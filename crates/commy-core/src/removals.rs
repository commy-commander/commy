use crate::ids::UserId;
use crate::server::Server;
use serde::{Deserialize, Serialize};

pub const MAX_KICKS_KEPT: usize = 200;
pub const MAX_BAN_MS: i64 = 366 * 24 * 60 * 60 * 1000;
pub const BAN_HOURS: &[u32] = &[1, 5, 24, 72, 168, 336, 720, 2160];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BanRecord {
    #[serde(default)]
    pub by: Option<UserId>,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub until: Option<i64>,
}

impl BanRecord {
    pub fn ended(&self, now_ms: i64) -> bool {
        self.until.is_some_and(|until| now_ms >= until)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kick {
    pub user: UserId,
    pub by: UserId,
    pub at: i64,
}

pub fn valid_term(at: i64, until: Option<i64>) -> bool {
    until.is_none_or(|until| until > at && until - at <= MAX_BAN_MS)
}

pub fn term_ms(hours: u32) -> Option<i64> {
    BAN_HOURS.contains(&hours).then(|| i64::from(hours) * 3_600_000)
}

pub fn record_kick(server: &mut Server, kick: Kick) -> bool {
    if server.kicked.iter().any(|k| k.user == kick.user && k.at >= kick.at) {
        return false;
    }
    server.kicked.retain(|k| k.user != kick.user);
    server.kicked.push(kick);
    server.kicked.sort_by_key(|k| (std::cmp::Reverse(k.at), k.user));
    server.kicked.truncate(MAX_KICKS_KEPT);
    true
}

pub fn forget_kick(server: &mut Server, user: UserId) {
    server.kicked.retain(|k| k.user != user);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ban_ends_at_its_time_and_an_open_ban_never_does() {
        let open = BanRecord { by: None, at: 0, until: None };
        let timed = BanRecord { by: None, at: 0, until: Some(1_000) };
        assert!(!open.ended(i64::MAX));
        assert!(!timed.ended(999));
        assert!(timed.ended(1_000));
    }

    #[test]
    fn terms_run_forward_and_at_most_a_year() {
        assert!(valid_term(10, None));
        assert!(valid_term(10, Some(11)));
        assert!(!valid_term(10, Some(10)));
        assert!(!valid_term(10, Some(5)));
        assert!(valid_term(0, Some(MAX_BAN_MS)));
        assert!(!valid_term(0, Some(MAX_BAN_MS + 1)));
        assert_eq!(term_ms(5), Some(5 * 3_600_000));
        assert_eq!(term_ms(720), Some(30 * 86_400_000));
        assert_eq!(term_ms(2), None);
    }

    #[test]
    fn the_kick_list_keeps_each_person_once_newest_first_and_bounded() {
        let mut server = Server::new("s", UserId::new());
        let (a, b, by) = (UserId::new(), UserId::new(), UserId::new());
        assert!(record_kick(&mut server, Kick { user: a, by, at: 10 }));
        assert!(record_kick(&mut server, Kick { user: b, by, at: 20 }));
        assert!(!record_kick(&mut server, Kick { user: a, by, at: 5 }));
        assert!(record_kick(&mut server, Kick { user: a, by, at: 30 }));
        assert_eq!(server.kicked.iter().map(|k| (k.user, k.at)).collect::<Vec<_>>(), vec![(a, 30), (b, 20)]);
        forget_kick(&mut server, a);
        assert_eq!(server.kicked.len(), 1);
        for at in 0..(MAX_KICKS_KEPT as i64 + 10) {
            record_kick(&mut server, Kick { user: UserId::new(), by, at: 100 + at });
        }
        assert_eq!(server.kicked.len(), MAX_KICKS_KEPT);
        assert!(server.kicked.windows(2).all(|w| w[0].at >= w[1].at));
    }
}
