use commy_core::Presence;

pub const DEFAULT_TIMEOUT_MINUTES: u32 = 10;

pub const MAX_TIMEOUT_MINUTES: u32 = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfkAction {
    GoIdle,
    Restore,
    None,
}

pub fn clamp_timeout_minutes(minutes: u32) -> u32 {
    minutes.min(MAX_TIMEOUT_MINUTES)
}

pub fn decide(idle_secs: i64, timeout_secs: i64, presence: Presence, auto_idle: bool) -> AfkAction {
    let timed_out = timeout_secs > 0 && idle_secs >= timeout_secs;
    if timed_out {
        if presence == Presence::Online && !auto_idle {
            return AfkAction::GoIdle;
        }
    } else if auto_idle {
        return AfkAction::Restore;
    }
    AfkAction::None
}

pub fn voice_timeout_secs(personal_secs: i64, server_minutes: Option<u32>) -> i64 {
    let server = server_minutes.map_or(0, |m| m as i64 * 60);
    match (personal_secs > 0, server > 0) {
        (true, true) => personal_secs.min(server),
        (true, false) => personal_secs,
        (false, true) => server,
        (false, false) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorter_voice_timeout_wins() {
        assert_eq!(voice_timeout_secs(600, Some(5)), 300);
        assert_eq!(voice_timeout_secs(120, Some(5)), 120);
        assert_eq!(voice_timeout_secs(0, Some(1)), 60, "server limit applies even with yours off");
        assert_eq!(voice_timeout_secs(600, None), 600);
        assert_eq!(voice_timeout_secs(0, None), 0);
    }

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

    #[test]
    fn timeout_is_clamped() {
        assert_eq!(clamp_timeout_minutes(0), 0);
        assert_eq!(clamp_timeout_minutes(10), 10);
        assert_eq!(clamp_timeout_minutes(10_000), MAX_TIMEOUT_MINUTES);
    }
}
