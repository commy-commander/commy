use commy_core::lock_or_recover::LockOrRecover;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const FORGET_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub burst: f64,
    pub per_second: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub connections_per_address: usize,
    pub connections_total: usize,
    pub appends: Rate,
    pub connection_appends: Rate,
    pub opens: Rate,
    pub address_opens: Rate,
    pub pictures: Rate,
    pub stream_appends: Rate,
    pub new_streams: Rate,
    pub passed_new_streams: Rate,
    pub wakes: Rate,
    pub signals: Rate,
    pub address_signals: Rate,
    pub wills: Rate,
    pub reports: Rate,
    pub daily_bytes_per_address: u64,
    pub daily_bytes_total: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connections_per_address: 1024,
            connections_total: 12_000,
            appends: Rate { burst: 300.0, per_second: 20.0 },
            connection_appends: Rate { burst: 60.0, per_second: 3.0 },
            opens: Rate { burst: 40.0, per_second: 4.0 },
            address_opens: Rate { burst: 600.0, per_second: 60.0 },
            pictures: Rate { burst: 200.0, per_second: 20.0 },
            stream_appends: Rate { burst: 100.0, per_second: 10.0 },
            new_streams: Rate { burst: 30.0, per_second: 1.0 / 120.0 },
            passed_new_streams: Rate { burst: 300.0, per_second: 0.5 },
            wakes: Rate { burst: 120.0, per_second: 1.0 },
            signals: Rate { burst: 60.0, per_second: 2.0 },
            address_signals: Rate { burst: 1200.0, per_second: 40.0 },
            wills: Rate { burst: 32.0, per_second: 0.5 },
            reports: Rate { burst: 5.0, per_second: 1.0 / 600.0 },
            daily_bytes_per_address: 64 * 1024 * 1024,
            daily_bytes_total: 16 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AddressKey(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionKey(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Caller {
    pub address: AddressKey,
    pub connection: ConnectionKey,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(rate: Rate, now: Instant) -> Self {
        Self { tokens: rate.burst, at: now }
    }

    fn take(&mut self, rate: Rate, now: Instant) -> Result<(), Duration> {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate.per_second).min(rate.burst);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - self.tokens) / rate.per_second))
        }
    }
}

#[derive(Debug)]
struct Address {
    connections: usize,
    appends: Bucket,
    opens: Bucket,
    pictures: Bucket,
    new_streams: Bucket,
    passed_new_streams: Bucket,
    wakes: Bucket,
    signals: Bucket,
    reports: Bucket,
    day_started: Instant,
    day_bytes: u64,
    seen: Instant,
}

#[derive(Debug)]
struct Connection {
    appends: Bucket,
    opens: Bucket,
    signals: Bucket,
    wills: Bucket,
}

#[derive(Debug, Default)]
struct Books {
    addresses: HashMap<AddressKey, Address>,
    connections: HashMap<ConnectionKey, Connection>,
    streams: HashMap<[u8; 32], (Bucket, Instant)>,
    total: Option<(Instant, u64)>,
}

pub struct Limiter {
    secret: [u8; 32],
    limits: Limits,
    books: Mutex<Books>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Wait(Duration),
    Full,
}

const IPV6_NETWORK_BYTES: usize = 8;

fn network_of(mut octets: [u8; 16]) -> [u8; 16] {
    octets[IPV6_NETWORK_BYTES..].fill(0);
    octets
}

impl AddressKey {
    pub fn holder(self) -> i64 {
        self.0 as i64
    }
}

impl Limiter {
    pub fn new(limits: Limits) -> Self {
        Self { secret: rand::random(), limits, books: Mutex::new(Books::default()) }
    }

    pub fn key(&self, address: IpAddr) -> AddressKey {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secret).expect("HMAC takes any key length");
        let bytes = match address {
            IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or_else(|| network_of(v6.octets()), |v4| v4.to_ipv6_mapped().octets()),
        };
        mac.update(&bytes);
        let out = mac.finalize().into_bytes();
        AddressKey(u64::from_le_bytes(out[..8].try_into().expect("8 bytes")))
    }

    fn address<'a>(&self, books: &'a mut Books, key: AddressKey, now: Instant) -> &'a mut Address {
        let limits = &self.limits;
        let entry = books.addresses.entry(key).or_insert_with(|| Address {
            connections: 0,
            appends: Bucket::full(limits.appends, now),
            opens: Bucket::full(limits.address_opens, now),
            pictures: Bucket::full(limits.pictures, now),
            new_streams: Bucket::full(limits.new_streams, now),
            passed_new_streams: Bucket::full(limits.passed_new_streams, now),
            wakes: Bucket::full(limits.wakes, now),
            signals: Bucket::full(limits.address_signals, now),
            reports: Bucket::full(limits.reports, now),
            day_started: now,
            day_bytes: 0,
            seen: now,
        });
        entry.seen = now;
        entry
    }

    pub fn connect(&self, key: AddressKey) -> Result<Caller, Refusal> {
        let now = Instant::now();
        let mut books = self.books.lock_or_recover();
        if books.connections.len() >= self.limits.connections_total {
            return Err(Refusal::Full);
        }
        let address = self.address(&mut books, key, now);
        if address.connections >= self.limits.connections_per_address {
            return Err(Refusal::Full);
        }
        address.connections += 1;
        let connection = self.fresh_connection(&mut books, now);
        Ok(Caller { address: key, connection })
    }

    fn fresh_connection(&self, books: &mut Books, now: Instant) -> ConnectionKey {
        let limits = &self.limits;
        loop {
            let key = ConnectionKey(rand::random());
            if let std::collections::hash_map::Entry::Vacant(slot) = books.connections.entry(key) {
                slot.insert(Connection {
                    appends: Bucket::full(limits.connection_appends, now),
                    opens: Bucket::full(limits.opens, now),
                    signals: Bucket::full(limits.signals, now),
                    wills: Bucket::full(limits.wills, now),
                });
                return key;
            }
        }
    }

    pub fn disconnect(&self, caller: Caller) {
        let mut books = self.books.lock_or_recover();
        books.connections.remove(&caller.connection);
        if let Some(address) = books.addresses.get_mut(&caller.address) {
            address.connections = address.connections.saturating_sub(1);
            address.seen = Instant::now();
        }
    }

    pub fn open(&self, caller: Caller) -> Result<(), Refusal> {
        self.open_at(caller, Instant::now())
    }

    fn open_at(&self, caller: Caller, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock_or_recover();
        let limits = self.limits.clone();
        let connection = books.connections.get_mut(&caller.connection).ok_or(Refusal::Full)?;
        connection.opens.take(limits.opens, now).map_err(Refusal::Wait)?;
        self.address(&mut books, caller.address, now).opens.take(limits.address_opens, now).map_err(Refusal::Wait)
    }

    pub fn picture(&self, key: AddressKey) -> Result<(), Refusal> {
        let now = Instant::now();
        let mut books = self.books.lock_or_recover();
        let rate = self.limits.pictures;
        self.address(&mut books, key, now).pictures.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn new_stream(&self, key: AddressKey) -> Result<(), Refusal> {
        self.new_stream_at(key, Instant::now())
    }

    fn new_stream_at(&self, key: AddressKey, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock_or_recover();
        let rate = self.limits.new_streams;
        self.address(&mut books, key, now).new_streams.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn passed_new_stream(&self, key: AddressKey) -> Result<(), Refusal> {
        self.passed_new_stream_at(key, Instant::now())
    }

    fn passed_new_stream_at(&self, key: AddressKey, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock_or_recover();
        let rate = self.limits.passed_new_streams;
        self.address(&mut books, key, now).passed_new_streams.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn wake(&self, key: AddressKey) -> Result<(), Refusal> {
        let now = Instant::now();
        let mut books = self.books.lock_or_recover();
        let rate = self.limits.wakes;
        self.address(&mut books, key, now).wakes.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn report(&self, key: AddressKey) -> Result<(), Refusal> {
        let now = Instant::now();
        let mut books = self.books.lock_or_recover();
        let rate = self.limits.reports;
        self.address(&mut books, key, now).reports.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn append_from(&self, caller: Caller, stream: &[u8; 32], bytes: usize) -> Result<(), Refusal> {
        self.append_from_at(caller, stream, bytes, Instant::now())
    }

    fn append_from_at(&self, caller: Caller, stream: &[u8; 32], bytes: usize, now: Instant) -> Result<(), Refusal> {
        {
            let mut books = self.books.lock_or_recover();
            let rate = self.limits.connection_appends;
            let connection = books.connections.get_mut(&caller.connection).ok_or(Refusal::Full)?;
            connection.appends.take(rate, now).map_err(Refusal::Wait)?;
        }
        self.append_at(caller.address, stream, bytes, now)
    }

    pub fn append(&self, key: AddressKey, stream: &[u8; 32], bytes: usize) -> Result<(), Refusal> {
        self.append_at(key, stream, bytes, Instant::now())
    }

    fn append_at(&self, key: AddressKey, stream: &[u8; 32], bytes: usize, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock_or_recover();
        let limits = self.limits.clone();
        let address = self.address(&mut books, key, now);
        if now.saturating_duration_since(address.day_started) >= DAY {
            address.day_started = now;
            address.day_bytes = 0;
        }
        if address.day_bytes + bytes as u64 > limits.daily_bytes_per_address {
            let left = DAY.saturating_sub(now.saturating_duration_since(address.day_started));
            return Err(Refusal::Wait(left));
        }
        address.appends.take(limits.appends, now).map_err(Refusal::Wait)?;
        address.day_bytes += bytes as u64;
        let (since, spent) = books.total.get_or_insert((now, 0));
        if now.saturating_duration_since(*since) >= DAY {
            (*since, *spent) = (now, 0);
        }
        if *spent + bytes as u64 > limits.daily_bytes_total {
            return Err(Refusal::Wait(DAY.saturating_sub(now.saturating_duration_since(*since))));
        }
        *spent += bytes as u64;
        let (bucket, seen) = books.streams.entry(*stream).or_insert_with(|| (Bucket::full(limits.stream_appends, now), now));
        *seen = now;
        bucket.take(limits.stream_appends, now).map_err(Refusal::Wait)
    }

    pub fn signal(&self, caller: Caller, bytes: usize) -> Result<(), Refusal> {
        self.signal_at(caller, bytes, Instant::now())
    }

    fn signal_at(&self, caller: Caller, bytes: usize, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock_or_recover();
        let limits = self.limits.clone();
        let connection = books.connections.get_mut(&caller.connection).ok_or(Refusal::Full)?;
        connection.signals.take(limits.signals, now).map_err(Refusal::Wait)?;
        let address = self.address(&mut books, caller.address, now);
        if now.saturating_duration_since(address.day_started) >= DAY {
            address.day_started = now;
            address.day_bytes = 0;
        }
        if address.day_bytes + bytes as u64 > limits.daily_bytes_per_address {
            return Err(Refusal::Wait(DAY.saturating_sub(now.saturating_duration_since(address.day_started))));
        }
        address.signals.take(limits.address_signals, now).map_err(Refusal::Wait)?;
        address.day_bytes += bytes as u64;
        Ok(())
    }

    pub fn will(&self, caller: Caller) -> Result<(), Refusal> {
        self.will_at(caller, Instant::now())
    }

    fn will_at(&self, caller: Caller, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock_or_recover();
        let rate = self.limits.wills;
        let connection = books.connections.get_mut(&caller.connection).ok_or(Refusal::Full)?;
        connection.wills.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn forget_idle(&self) -> usize {
        let now = Instant::now();
        let mut books = self.books.lock_or_recover();
        let before = books.addresses.len() + books.streams.len();
        books.addresses.retain(|_, a| {
            a.connections > 0 || now.saturating_duration_since(a.seen) < FORGET_AFTER || now.saturating_duration_since(a.day_started) < DAY && a.day_bytes > 0
        });
        books.streams.retain(|_, (_, seen)| now.saturating_duration_since(*seen) < FORGET_AFTER);
        before - books.addresses.len() - books.streams.len()
    }

    pub fn tracked(&self) -> usize {
        self.books.lock_or_recover().addresses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tight() -> Limits {
        Limits {
            connections_per_address: 2,
            connections_total: 100,
            appends: Rate { burst: 3.0, per_second: 1.0 },
            connection_appends: Rate { burst: 2.0, per_second: 1.0 },
            opens: Rate { burst: 2.0, per_second: 1.0 },
            address_opens: Rate { burst: 3.0, per_second: 1.0 },
            pictures: Rate { burst: 2.0, per_second: 1.0 },
            stream_appends: Rate { burst: 100.0, per_second: 100.0 },
            new_streams: Rate { burst: 2.0, per_second: 1.0 },
            passed_new_streams: Rate { burst: 3.0, per_second: 1.0 },
            wakes: Rate { burst: 2.0, per_second: 1.0 },
            signals: Rate { burst: 2.0, per_second: 1.0 },
            address_signals: Rate { burst: 3.0, per_second: 1.0 },
            wills: Rate { burst: 2.0, per_second: 1.0 },
            reports: Rate { burst: 2.0, per_second: 0.001 },
            daily_bytes_per_address: 10_000,
            daily_bytes_total: 25_000,
        }
    }

    #[test]
    fn the_whole_service_has_a_daily_byte_budget() {
        let limiter = Limiter::new(Limits { appends: Rate { burst: 1000.0, per_second: 1000.0 }, ..tight() });
        let now = Instant::now();
        let stream = [7; 32];
        for n in 0..3u8 {
            let key = limiter.key(std::net::IpAddr::from([10, 0, 0, n]));
            let result = limiter.append_at(key, &stream, 9_000, now);
            assert_eq!(result.is_ok(), n < 2, "address {n}");
        }
    }

    #[test]
    fn live_signals_have_their_own_allowance_and_count_toward_the_day() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("192.0.2.9".parse().unwrap());
        let caller = limiter.connect(key).unwrap();
        assert!(limiter.signal(caller, 100).is_ok());
        assert!(limiter.signal(caller, 100).is_ok());
        assert!(matches!(limiter.signal(caller, 100), Err(Refusal::Wait(_))));
        assert!(limiter.append(key, &[1; 32], 100).is_ok(), "signals do not use up appends");
        let other = limiter.connect(limiter.key("192.0.2.10".parse().unwrap())).unwrap();
        assert!(matches!(limiter.signal(other, 20_000), Err(Refusal::Wait(_))));
    }

    #[test]
    fn each_connection_has_its_own_signal_allowance_under_the_address_ceiling() {
        let limiter = Limiter::new(Limits { connections_per_address: 10, ..tight() });
        let key = limiter.key("192.0.2.20".parse().unwrap());
        let now = Instant::now();
        let first = limiter.connect(key).unwrap();
        let second = limiter.connect(key).unwrap();
        assert!(limiter.signal_at(first, 10, now).is_ok());
        assert!(limiter.signal_at(first, 10, now).is_ok());
        assert!(limiter.signal_at(first, 10, now).is_err(), "one connection used its own allowance");
        assert!(limiter.signal_at(second, 10, now).is_ok(), "another connection from the same address still signals");
        assert!(limiter.signal_at(second, 10, now).is_err(), "the address ceiling still holds");
        assert!(limiter.signal_at(second, 10, now + Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn last_words_are_limited_per_connection_and_spend_no_signals_or_bytes() {
        let limiter = Limiter::new(Limits { daily_bytes_per_address: 50, ..tight() });
        let key = limiter.key("192.0.2.30".parse().unwrap());
        let caller = limiter.connect(key).unwrap();
        let now = Instant::now();
        assert!(limiter.will_at(caller, now).is_ok());
        assert!(limiter.will_at(caller, now).is_ok());
        assert!(limiter.will_at(caller, now).is_err());
        assert!(limiter.will_at(caller, now + Duration::from_secs(1)).is_ok());
        assert!(limiter.signal_at(caller, 40, now).is_ok(), "wills did not touch the signal allowance or the day's bytes");
    }

    #[test]
    fn a_closed_connection_takes_its_allowance_with_it() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("192.0.2.40".parse().unwrap());
        let caller = limiter.connect(key).unwrap();
        limiter.disconnect(caller);
        assert_eq!(limiter.open(caller), Err(Refusal::Full));
        assert_eq!(limiter.signal(caller, 1), Err(Refusal::Full));
        assert_eq!(limiter.will(caller), Err(Refusal::Full));
    }

    #[test]
    fn addresses_are_kept_only_as_keyed_hashes() {
        let a = Limiter::new(tight());
        let b = Limiter::new(tight());
        let ip: IpAddr = "198.51.100.7".parse().unwrap();
        assert_eq!(a.key(ip), a.key(ip));
        assert_ne!(a.key(ip), a.key("198.51.100.8".parse().unwrap()));
        assert_ne!(a.key(ip), b.key(ip), "each process hashes with its own secret");
        assert_eq!(a.key(ip), a.key("::ffff:198.51.100.7".parse().unwrap()));
    }

    #[test]
    fn one_ipv6_network_shares_one_allowance() {
        let limiter = Limiter::new(tight());
        let key = |ip: &str| limiter.key(ip.parse().unwrap());
        assert_eq!(key("2001:db8:1:2::1"), key("2001:db8:1:2:ffff::9"), "the same /64");
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
    }

    #[test]
    fn connections_are_capped_per_address() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.1".parse().unwrap());
        let first = limiter.connect(key).unwrap();
        assert!(limiter.connect(key).is_ok());
        assert_eq!(limiter.connect(key), Err(Refusal::Full));
        limiter.disconnect(first);
        assert!(limiter.connect(key).is_ok());
        let other = limiter.key("203.0.113.2".parse().unwrap());
        assert!(limiter.connect(other).is_ok());
    }

    #[test]
    fn the_whole_service_has_a_connection_ceiling() {
        let limiter = Limiter::new(Limits { connections_total: 3, ..tight() });
        let callers: Vec<Caller> = (1..=3u8).map(|n| limiter.connect(limiter.key(IpAddr::from([10, 1, 0, n]))).unwrap()).collect();
        let late = limiter.key("10.1.0.9".parse().unwrap());
        assert_eq!(limiter.connect(late), Err(Refusal::Full));
        limiter.disconnect(callers[0]);
        assert!(limiter.connect(late).is_ok());
    }

    #[test]
    fn appends_refill_over_time() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.1".parse().unwrap());
        let start = Instant::now();
        for _ in 0..3 {
            assert!(limiter.append_at(key, &[1; 32], 10, start).is_ok());
        }
        let Err(Refusal::Wait(wait)) = limiter.append_at(key, &[1; 32], 10, start) else { panic!("should be limited") };
        assert!(wait <= Duration::from_secs(1));
        assert!(limiter.append_at(key, &[1; 32], 10, start + Duration::from_millis(1100)).is_ok());
    }

    #[test]
    fn a_day_of_bytes_is_capped_per_address() {
        let limiter = Limiter::new(Limits { appends: Rate { burst: 1000.0, per_second: 1000.0 }, ..tight() });
        let key = limiter.key("203.0.113.9".parse().unwrap());
        let start = Instant::now();
        assert!(limiter.append_at(key, &[1; 32], 6_000, start).is_ok());
        let Err(Refusal::Wait(wait)) = limiter.append_at(key, &[2; 32], 6_000, start) else { panic!("over the day's bytes") };
        assert!(wait > Duration::from_secs(60 * 60));
        assert!(limiter.append_at(key, &[2; 32], 6_000, start + DAY).is_ok());
    }

    #[test]
    fn a_busy_stream_is_slowed_whoever_writes() {
        let limiter = Limiter::new(Limits {
            appends: Rate { burst: 100.0, per_second: 100.0 },
            stream_appends: Rate { burst: 2.0, per_second: 1.0 },
            ..tight()
        });
        let a = limiter.key("203.0.113.1".parse().unwrap());
        let b = limiter.key("203.0.113.2".parse().unwrap());
        let now = Instant::now();
        assert!(limiter.append_at(a, &[5; 32], 1, now).is_ok());
        assert!(limiter.append_at(b, &[5; 32], 1, now).is_ok());
        assert!(limiter.append_at(a, &[5; 32], 1, now).is_err());
        assert!(limiter.append_at(a, &[6; 32], 1, now).is_ok());
    }

    #[test]
    fn opens_are_rate_limited_per_connection_and_per_address() {
        let limiter = Limiter::new(Limits { connections_per_address: 10, ..tight() });
        let key = limiter.key("203.0.113.1".parse().unwrap());
        let now = Instant::now();
        let first = limiter.connect(key).unwrap();
        assert!(limiter.open_at(first, now).is_ok());
        assert!(limiter.open_at(first, now).is_ok());
        assert!(matches!(limiter.open_at(first, now), Err(Refusal::Wait(_))));
        let second = limiter.connect(key).unwrap();
        assert!(limiter.open_at(second, now).is_ok());
        assert!(matches!(limiter.open_at(second, now), Err(Refusal::Wait(_))), "the address ceiling");
        let elsewhere = limiter.connect(limiter.key("203.0.113.2".parse().unwrap())).unwrap();
        assert!(limiter.open_at(elsewhere, now).is_ok());
    }

    #[test]
    fn each_connection_has_its_own_message_allowance_under_the_address_ceiling() {
        let limiter = Limiter::new(Limits { connections_per_address: 10, ..tight() });
        let key = limiter.key("203.0.113.8".parse().unwrap());
        let now = Instant::now();
        let first = limiter.connect(key).unwrap();
        let second = limiter.connect(key).unwrap();
        assert!(limiter.append_from_at(first, &[1; 32], 10, now).is_ok());
        assert!(limiter.append_from_at(first, &[1; 32], 10, now).is_ok());
        assert!(limiter.append_from_at(first, &[1; 32], 10, now).is_err(), "one connection used its own allowance");
        assert!(limiter.append_from_at(second, &[2; 32], 10, now).is_ok());
        assert!(limiter.append_from_at(second, &[2; 32], 10, now).is_err(), "the address ceiling still holds");
        limiter.disconnect(first);
        assert_eq!(limiter.append_from_at(first, &[1; 32], 10, now), Err(Refusal::Full));
    }

    #[test]
    fn new_streams_are_limited_per_address() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.6".parse().unwrap());
        let now = Instant::now();
        assert!(limiter.new_stream_at(key, now).is_ok());
        assert!(limiter.new_stream_at(key, now).is_ok());
        assert!(matches!(limiter.new_stream_at(key, now), Err(Refusal::Wait(_))));
        assert!(limiter.append_at(key, &[9; 32], 10, now).is_ok(), "writing to streams that exist is not affected");
        assert!(limiter.new_stream_at(key, now + Duration::from_secs(1)).is_ok());
        assert!(limiter.new_stream_at(limiter.key("203.0.113.7".parse().unwrap()), now).is_ok());
        for _ in 0..3 {
            assert!(limiter.passed_new_stream_at(key, now).is_ok(), "passes from many accounts behind one address go further");
        }
        assert!(matches!(limiter.passed_new_stream_at(key, now), Err(Refusal::Wait(_))));
    }

    #[test]
    fn pictures_have_their_own_allowance() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.5".parse().unwrap());
        let caller = limiter.connect(key).unwrap();
        assert!(limiter.picture(key).is_ok());
        assert!(limiter.picture(key).is_ok());
        assert!(limiter.picture(key).is_err());
        assert!(limiter.open(caller).is_ok(), "pictures do not use up opens");
    }

    #[test]
    fn idle_addresses_are_forgotten() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.1".parse().unwrap());
        limiter.connect(key).unwrap();
        assert_eq!(limiter.forget_idle(), 0);
        assert_eq!(limiter.tracked(), 1);
    }
}
