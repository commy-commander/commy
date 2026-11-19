use std::time::Duration;

pub const STALL_AFTER: Duration = Duration::from_secs(45);
pub const NOT_STARTED: &str = "Your servers have not started loading in this browser.";
const ACCOUNT_WAITING: &str = "Your server list is taking longer than usual to load.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    NotStarted,
    AccountWaiting,
    Live,
}

pub fn reason(state: SyncState) -> Option<&'static str> {
    match state {
        SyncState::NotStarted => Some(NOT_STARTED),
        SyncState::AccountWaiting => Some(ACCOUNT_WAITING),
        SyncState::Live => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stalled_start_names_what_is_stuck_and_a_live_one_says_nothing() {
        assert_eq!(reason(SyncState::NotStarted), Some(NOT_STARTED));
        assert!(reason(SyncState::AccountWaiting).is_some_and(|text| text.contains("server list")));
        assert_eq!(reason(SyncState::Live), None);
    }

    #[test]
    fn the_watch_waits_long_enough_for_a_slow_connection() {
        assert!(STALL_AFTER >= Duration::from_secs(30));
    }
}
