use std::time::Duration;

pub const LINK_GRACE: Duration = Duration::from_secs(2);

pub fn same_link(link: Option<(&str, Duration)>, node: &str) -> bool {
    link.is_some_and(|(linked, _)| linked.eq_ignore_ascii_case(node))
}

pub fn link_outlived(link: Option<(&str, Duration)>, here: bool, node: &str, link_join: Option<i64>, since: Option<i64>) -> bool {
    let Some((_, age)) = link.filter(|_| same_link(link, node)) else { return false };
    let rejoined = matches!((link_join, since), (Some(before), Some(now)) if before != now);
    !here || (rejoined && age > LINK_GRACE)
}

pub fn new_join(answered: Option<i64>, here: bool, since: Option<i64>) -> bool {
    here && since.is_some() && answered != since
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_ends_when_its_device_leaves_or_joins_again() {
        let (desktop, phone) = ("ab".repeat(32), "cd".repeat(32));
        let old = Some((desktop.as_str(), Duration::from_secs(60)));
        let recent = Some((desktop.as_str(), Duration::from_secs(6)));
        let fresh = Some((desktop.as_str(), Duration::from_secs(1)));
        assert!(link_outlived(old, false, &desktop.to_uppercase(), Some(1_000), None), "the linked device says it is not in our room");
        assert!(!link_outlived(old, false, &phone, Some(1_000), None), "another device of theirs is not in voice");
        assert!(!link_outlived(old, true, &desktop, Some(1_000), Some(1_000)), "the seat repeated");
        assert!(link_outlived(old, true, &desktop, Some(1_000), Some(9_000)), "the app restarted and joined again");
        assert!(link_outlived(recent, true, &desktop, Some(1_000), Some(9_000)), "the app restarted seconds after the link was made");
        assert!(!link_outlived(fresh, true, &desktop, Some(1_000), Some(9_000)), "the link already belongs to the new join");
        assert!(!link_outlived(old, true, &desktop, None, Some(9_000)), "the first join we hear of");
        assert!(!link_outlived(None, false, &desktop, Some(1_000), None), "no link");
        assert!(same_link(old, &desktop.to_uppercase()));
        assert!(!same_link(old, &phone), "a link to another of their devices");
        assert!(!same_link(None, &desktop));
    }

    #[test]
    fn a_join_into_our_room_is_answered_once() {
        assert!(new_join(None, true, Some(1_000)), "the first join we hear of");
        assert!(!new_join(Some(1_000), true, Some(1_000)), "their seat repeated");
        assert!(new_join(Some(1_000), true, Some(9_000)), "they restarted and joined again");
        assert!(!new_join(None, false, Some(1_000)), "they joined another room");
        assert!(!new_join(None, true, None), "an announce without a join");
    }
}
