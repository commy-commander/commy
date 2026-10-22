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
    pub appends: Rate,
    pub opens: Rate,
    pub stream_appends: Rate,
    pub daily_bytes_per_address: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connections_per_address: 256,
            appends: Rate { burst: 60.0, per_second: 5.0 },
            opens: Rate { burst: 200.0, per_second: 20.0 },
            stream_appends: Rate { burst: 100.0, per_second: 20.0 },
            daily_bytes_per_address: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AddressKey(u64);

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
    day_started: Instant,
    day_bytes: u64,
    seen: Instant,
}

#[derive(Debug, Default)]
struct Books {
    addresses: HashMap<AddressKey, Address>,
    streams: HashMap<[u8; 32], (Bucket, Instant)>,
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

impl Limiter {
    pub fn new(limits: Limits) -> Self {
        Self { secret: rand::random(), limits, books: Mutex::new(Books::default()) }
    }

    pub fn key(&self, address: IpAddr) -> AddressKey {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secret).expect("HMAC takes any key length");
        let bytes = match address {
            IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
            IpAddr::V6(v6) => v6.octets(),
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
            opens: Bucket::full(limits.opens, now),
            day_started: now,
            day_bytes: 0,
            seen: now,
        });
        entry.seen = now;
        entry
    }

    pub fn connect(&self, key: AddressKey) -> Result<(), Refusal> {
        let now = Instant::now();
        let mut books = self.books.lock().unwrap();
        let address = self.address(&mut books, key, now);
        if address.connections >= self.limits.connections_per_address {
            return Err(Refusal::Full);
        }
        address.connections += 1;
        Ok(())
    }

    pub fn disconnect(&self, key: AddressKey) {
        let mut books = self.books.lock().unwrap();
        if let Some(address) = books.addresses.get_mut(&key) {
            address.connections = address.connections.saturating_sub(1);
            address.seen = Instant::now();
        }
    }

    pub fn open(&self, key: AddressKey) -> Result<(), Refusal> {
        let now = Instant::now();
        let mut books = self.books.lock().unwrap();
        let rate = self.limits.opens;
        self.address(&mut books, key, now).opens.take(rate, now).map_err(Refusal::Wait)
    }

    pub fn append(&self, key: AddressKey, stream: &[u8; 32], bytes: usize) -> Result<(), Refusal> {
        self.append_at(key, stream, bytes, Instant::now())
    }

    fn append_at(&self, key: AddressKey, stream: &[u8; 32], bytes: usize, now: Instant) -> Result<(), Refusal> {
        let mut books = self.books.lock().unwrap();
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
        let (bucket, seen) = books.streams.entry(*stream).or_insert_with(|| (Bucket::full(limits.stream_appends, now), now));
        *seen = now;
        bucket.take(limits.stream_appends, now).map_err(Refusal::Wait)
    }

    pub fn forget_idle(&self) -> usize {
        let now = Instant::now();
        let mut books = self.books.lock().unwrap();
        let before = books.addresses.len() + books.streams.len();
        books.addresses.retain(|_, a| {
            a.connections > 0 || now.saturating_duration_since(a.seen) < FORGET_AFTER || now.saturating_duration_since(a.day_started) < DAY && a.day_bytes > 0
        });
        books.streams.retain(|_, (_, seen)| now.saturating_duration_since(*seen) < FORGET_AFTER);
        before - books.addresses.len() - books.streams.len()
    }

    pub fn tracked(&self) -> usize {
        self.books.lock().unwrap().addresses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tight() -> Limits {
        Limits {
            connections_per_address: 2,
            appends: Rate { burst: 3.0, per_second: 1.0 },
            opens: Rate { burst: 2.0, per_second: 1.0 },
            stream_appends: Rate { burst: 100.0, per_second: 100.0 },
            daily_bytes_per_address: 10_000,
        }
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
    fn connections_are_capped_per_address() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.1".parse().unwrap());
        assert!(limiter.connect(key).is_ok());
        assert!(limiter.connect(key).is_ok());
        assert_eq!(limiter.connect(key), Err(Refusal::Full));
        limiter.disconnect(key);
        assert!(limiter.connect(key).is_ok());
        let other = limiter.key("203.0.113.2".parse().unwrap());
        assert!(limiter.connect(other).is_ok());
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
    fn opens_are_rate_limited() {
        let limiter = Limiter::new(tight());
        let key = limiter.key("203.0.113.1".parse().unwrap());
        assert!(limiter.open(key).is_ok());
        assert!(limiter.open(key).is_ok());
        assert!(matches!(limiter.open(key), Err(Refusal::Wait(_))));
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
