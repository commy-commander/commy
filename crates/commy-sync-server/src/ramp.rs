use serde::Serialize;

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const SETTLED_AFTER_DAYS: i64 = 7;
pub const TRUSTED_AFTER_DAYS: i64 = 30;
pub const FLOOD_FACTOR: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    New,
    Settled,
    Trusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Caps {
    pub pushes: u64,
    pub new_phones: u64,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::New, Tier::Settled, Tier::Trusted];

    pub fn caps(self) -> Caps {
        match self {
            Tier::New => Caps { pushes: 500, new_phones: 25 },
            Tier::Settled => Caps { pushes: 5_000, new_phones: 250 },
            Tier::Trusted => Caps { pushes: 50_000, new_phones: 2_500 },
        }
    }

    pub fn earned(clean_since_ms: i64, now_ms: i64) -> Tier {
        let clean_days = now_ms.saturating_sub(clean_since_ms) / DAY_MS;
        if clean_days >= TRUSTED_AFTER_DAYS {
            Tier::Trusted
        } else if clean_days >= SETTLED_AFTER_DAYS {
            Tier::Settled
        } else {
            Tier::New
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::New => "new",
            Tier::Settled => "settled",
            Tier::Trusted => "trusted",
        }
    }

    pub fn parse(text: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|tier| tier.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CutOff {
    pub at: i64,
    pub reason: String,
    pub automatic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerRecord {
    pub origin: String,
    pub first_seen: i64,
    pub clean_since: i64,
    pub cut_off: Option<CutOff>,
    pub pinned: Option<Tier>,
}

impl ServerRecord {
    pub fn first(origin: &str, now_ms: i64) -> Self {
        Self { origin: origin.to_string(), first_seen: now_ms, clean_since: now_ms, cut_off: None, pinned: None }
    }

    pub fn tier(&self, now_ms: i64) -> Tier {
        self.pinned.unwrap_or_else(|| Tier::earned(self.clean_since, now_ms))
    }

    pub fn cut(&mut self, now_ms: i64, reason: &str, automatic: bool) {
        self.cut_off = Some(CutOff { at: now_ms, reason: reason.chars().filter(|c| !c.is_control()).take(200).collect(), automatic });
    }

    pub fn restore(&mut self, now_ms: i64) {
        self.cut_off = None;
        self.clean_since = now_ms;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Day {
    pub pushes: u64,
    pub refused: u64,
    pub new_phones: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Send,
    OverPushCap,
    OverNewPhoneCap,
    OverBudget,
}

pub fn judge(caps: Caps, day: &Day, new_phone: bool, budget_left: u64) -> Verdict {
    if budget_left == 0 {
        Verdict::OverBudget
    } else if day.pushes >= caps.pushes {
        Verdict::OverPushCap
    } else if new_phone && day.new_phones >= caps.new_phones {
        Verdict::OverNewPhoneCap
    } else {
        Verdict::Send
    }
}

pub fn floods(caps: Caps, day: &Day) -> bool {
    day.refused >= caps.pushes.saturating_mul(FLOOD_FACTOR)
}

pub fn left(caps: Caps, day: &Day) -> Caps {
    Caps { pushes: caps.pushes.saturating_sub(day.pushes), new_phones: caps.new_phones.saturating_sub(day.new_phones) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_earns_higher_caps_with_clean_days() {
        assert_eq!(Tier::earned(0, 0), Tier::New);
        assert_eq!(Tier::earned(0, SETTLED_AFTER_DAYS * DAY_MS - 1), Tier::New);
        assert_eq!(Tier::earned(0, SETTLED_AFTER_DAYS * DAY_MS), Tier::Settled);
        assert_eq!(Tier::earned(0, TRUSTED_AFTER_DAYS * DAY_MS), Tier::Trusted);
        assert_eq!(Tier::earned(10, 0), Tier::New, "a clock that went backwards stays new");
        let caps: Vec<u64> = Tier::ALL.iter().map(|t| t.caps().pushes).collect();
        assert!(caps.windows(2).all(|w| w[0] < w[1]));
        let phones: Vec<u64> = Tier::ALL.iter().map(|t| t.caps().new_phones).collect();
        assert!(phones.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn being_cut_off_starts_the_ramp_over() {
        let mut record = ServerRecord::first("https://home.example", 0);
        let later = TRUSTED_AFTER_DAYS * DAY_MS;
        assert_eq!(record.tier(later), Tier::Trusted);
        record.cut(later, "flooded\npast its cap", true);
        assert_eq!(record.cut_off.as_ref().unwrap().reason, "floodedpast its cap");
        record.restore(later);
        assert_eq!(record.cut_off, None);
        assert_eq!(record.tier(later), Tier::New);
        assert_eq!(record.tier(later + SETTLED_AFTER_DAYS * DAY_MS), Tier::Settled);
    }

    #[test]
    fn an_operator_can_pin_a_tier() {
        let mut record = ServerRecord::first("https://home.example", 0);
        record.pinned = Some(Tier::Trusted);
        assert_eq!(record.tier(0), Tier::Trusted);
        record.pinned = Some(Tier::New);
        assert_eq!(record.tier(TRUSTED_AFTER_DAYS * DAY_MS), Tier::New);
        assert_eq!(Tier::parse("settled"), Some(Tier::Settled));
        assert_eq!(Tier::parse("gold"), None);
    }

    #[test]
    fn pushes_stop_at_the_daily_cap_and_new_phones_at_theirs() {
        let caps = Caps { pushes: 3, new_phones: 1 };
        let mut day = Day::default();
        assert_eq!(judge(caps, &day, true, 100), Verdict::Send);
        day.pushes = 1;
        day.new_phones = 1;
        assert_eq!(judge(caps, &day, true, 100), Verdict::OverNewPhoneCap, "no more strangers today");
        assert_eq!(judge(caps, &day, false, 100), Verdict::Send, "phones it already reached still get pings");
        day.pushes = 3;
        assert_eq!(judge(caps, &day, false, 100), Verdict::OverPushCap);
        assert_eq!(judge(caps, &Day::default(), false, 0), Verdict::OverBudget, "our own budget comes first");
        assert_eq!(left(caps, &day), Caps { pushes: 0, new_phones: 0 });
    }

    #[test]
    fn a_flood_is_many_times_the_cap() {
        let caps = Caps { pushes: 10, new_phones: 1 };
        assert!(!floods(caps, &Day { refused: 99, ..Day::default() }));
        assert!(floods(caps, &Day { refused: 100, ..Day::default() }));
    }
}
