use commy_core::Presence;

pub use commy_view::afk::{clamp_timeout_minutes, DEFAULT_TIMEOUT_MINUTES};

const NO_TIMEOUT: i64 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfkAction {
    GoIdle,
    Restore,
    None,
}

pub fn decide(idle_secs: i64, timeout_secs: i64, presence: Presence, auto_idle: bool) -> AfkAction {
    let timed_out = timeout_secs > NO_TIMEOUT && idle_secs >= timeout_secs;
    if timed_out {
        if presence == Presence::Online && !auto_idle {
            return AfkAction::GoIdle;
        }
    } else if auto_idle {
        return AfkAction::Restore;
    }
    AfkAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_online_before_timeout() {
        assert_eq!(decide(599, 600, Presence::Online, false), AfkAction::None);
    }

    #[test]
    fn goes_idle_at_timeout() {
        assert_eq!(decide(600, 600, Presence::Online, false), AfkAction::GoIdle);
    }

    #[test]
    fn does_not_repeat_once_idle() {
        assert_eq!(decide(900, 600, Presence::Idle, true), AfkAction::None);
    }

    #[test]
    fn restores_when_activity_resumes() {
        assert_eq!(decide(3, 600, Presence::Idle, true), AfkAction::Restore);
    }

    #[test]
    fn never_overrides_manual_presence() {
        for presence in [Presence::Idle, Presence::DoNotDisturb, Presence::Invisible] {
            assert_eq!(decide(10_000, 600, presence, false), AfkAction::None);
            assert_eq!(decide(1, 600, presence, false), AfkAction::None);
        }
    }

    #[test]
    fn zero_timeout_disables_afk() {
        assert_eq!(decide(1_000_000, 0, Presence::Online, false), AfkAction::None);
    }

    #[test]
    fn disabling_while_idle_restores() {
        assert_eq!(decide(1_000_000, 0, Presence::Idle, true), AfkAction::Restore);
    }
}
