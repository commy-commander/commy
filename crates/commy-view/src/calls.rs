use commy_core::{CallSignal, ChannelId, UserId};
use serde::Serialize;

pub const RING_FOR_MS: i64 = 45_000;

pub const REDIAL_EVERY_MS: i64 = 5_000;

#[derive(Debug, Clone, PartialEq)]
pub struct DmCall {
    pub partner: UserId,
    pub channel: ChannelId,
    pub node: Option<String>,
    pub outgoing: bool,
    pub active: bool,
    pub since_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DmCallView {
    pub state: String,
    pub user: String,
    pub name: String,
    pub outgoing: bool,
    pub seconds: u64,
    pub reason: Option<String>,
}

impl DmCall {
    pub fn view(&self, name: String, state: &str, reason: Option<&str>, now_ms: i64) -> DmCallView {
        DmCallView {
            state: state.to_string(),
            user: self.partner.to_string(),
            name,
            outgoing: self.outgoing,
            seconds: (now_ms - self.since_ms).max(0) as u64 / 1000,
            reason: reason.map(str::to_string),
        }
    }

    pub fn state(&self) -> &'static str {
        if self.active {
            "active"
        } else {
            "ringing"
        }
    }

    fn is(&self, partner: UserId, channel: ChannelId) -> bool {
        self.partner == partner && self.channel == channel
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallStep {
    Nothing,
    Reply(CallSignal),
    Ringing(DmCall),
    Connect(DmCall),
    Ended { call: DmCall, reason: &'static str },
}

#[derive(Debug, Default)]
pub struct CallBook {
    call: Option<DmCall>,
}

impl CallBook {
    pub fn current(&self) -> Option<&DmCall> {
        self.call.as_ref()
    }

    pub fn active(&self) -> Option<&DmCall> {
        self.call.as_ref().filter(|c| c.active)
    }

    pub fn start(&mut self, partner: UserId, now_ms: i64) -> Result<DmCall, String> {
        if self.call.is_some() {
            return Err("leave your current call or voice channel first".into());
        }
        let call = DmCall { partner, channel: ChannelId::new(), node: None, outgoing: true, active: false, since_ms: now_ms };
        self.call = Some(call.clone());
        Ok(call)
    }

    pub fn on_signal(&mut self, from: UserId, node: &str, channel: ChannelId, signal: CallSignal, now_ms: i64) -> CallStep {
        let matching = self.call.as_ref().is_some_and(|c| c.is(from, channel));
        match signal {
            CallSignal::Ring if matching => CallStep::Nothing,
            CallSignal::Ring if self.call.is_some() => CallStep::Reply(CallSignal::Decline),
            CallSignal::Ring => {
                let call = DmCall {
                    partner: from,
                    channel,
                    node: Some(node.to_string()),
                    outgoing: false,
                    active: false,
                    since_ms: now_ms,
                };
                self.call = Some(call.clone());
                CallStep::Ringing(call)
            }
            CallSignal::Accept => match self.call.as_mut().filter(|c| c.is(from, channel) && c.outgoing && !c.active) {
                Some(call) => {
                    call.node = Some(node.to_string());
                    CallStep::Connect(call.clone())
                }
                None => CallStep::Nothing,
            },
            CallSignal::Decline if matching => self.finish("declined"),
            CallSignal::HangUp if matching => {
                let active = self.call.as_ref().is_some_and(|c| c.active);
                self.finish(if active { "hung_up" } else { "missed" })
            }
            CallSignal::Decline | CallSignal::HangUp => CallStep::Nothing,
        }
    }

    pub fn settled_elsewhere(&mut self, partner: UserId, channel: ChannelId, signal: CallSignal) -> Option<DmCall> {
        if !matches!(signal, CallSignal::Accept | CallSignal::Decline) {
            return None;
        }
        let ringing = self.call.as_ref().is_some_and(|c| c.is(partner, channel) && !c.outgoing && !c.active);
        if ringing {
            self.call.take()
        } else {
            None
        }
    }

    pub fn answer(&mut self, accept: bool) -> Result<DmCall, String> {
        let call = self.call.clone().filter(|c| !c.outgoing && !c.active).ok_or("nobody is calling")?;
        if !accept {
            self.call = None;
        }
        Ok(call)
    }

    pub fn connected(&mut self, channel: ChannelId, now_ms: i64) -> Option<DmCall> {
        let call = self.call.as_mut().filter(|c| c.channel == channel)?;
        call.active = true;
        call.since_ms = now_ms;
        Some(call.clone())
    }

    pub fn end(&mut self) -> Option<DmCall> {
        self.call.take()
    }

    pub fn ring_out(&mut self, channel: ChannelId) -> CallStep {
        let stale = self.call.as_ref().is_some_and(|c| c.channel == channel && !c.active);
        if !stale {
            return CallStep::Nothing;
        }
        let outgoing = self.call.as_ref().is_some_and(|c| c.outgoing);
        self.finish(if outgoing { "unanswered" } else { "missed" })
    }

    pub fn admits(&self, user: UserId, channel: ChannelId, node_hex: &str) -> bool {
        self.active()
            .is_some_and(|c| c.is(user, channel) && c.node.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(node_hex)))
    }

    fn finish(&mut self, reason: &'static str) -> CallStep {
        match self.call.take() {
            Some(call) => CallStep::Ended { call, reason },
            None => CallStep::Nothing,
        }
    }
}

pub fn should_dial(our_node: &str, their_node: &str) -> bool {
    our_node < their_node
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEIRS: &str = "bb";
    const OTHER: &str = "cc";

    fn ringing(book: &mut CallBook, from: UserId) -> ChannelId {
        let channel = ChannelId::new();
        assert!(matches!(book.on_signal(from, THEIRS, channel, CallSignal::Ring, 0), CallStep::Ringing(_)));
        channel
    }

    #[test]
    fn a_ring_rings_once_and_a_second_caller_is_declined() {
        let mut book = CallBook::default();
        let (alice, bob) = (UserId::new(), UserId::new());
        let channel = ringing(&mut book, alice);
        assert_eq!(book.on_signal(alice, THEIRS, channel, CallSignal::Ring, 1), CallStep::Nothing);
        assert_eq!(book.on_signal(bob, OTHER, ChannelId::new(), CallSignal::Ring, 1), CallStep::Reply(CallSignal::Decline));
        assert_eq!(book.current().map(|c| c.partner), Some(alice));
    }

    #[test]
    fn answering_connects_to_the_device_that_rang() {
        let mut book = CallBook::default();
        let alice = UserId::new();
        let channel = ringing(&mut book, alice);
        let call = book.answer(true).unwrap();
        assert_eq!(call.node.as_deref(), Some(THEIRS));
        assert!(!book.admits(alice, channel, THEIRS));
        book.connected(channel, 10).unwrap();
        assert!(book.admits(alice, channel, THEIRS));
        assert!(book.admits(alice, channel, "BB"));
        assert!(!book.admits(alice, channel, OTHER));
        assert!(!book.admits(alice, ChannelId::new(), THEIRS));
    }

    #[test]
    fn declining_clears_the_call() {
        let mut book = CallBook::default();
        ringing(&mut book, UserId::new());
        assert!(book.answer(false).is_ok());
        assert!(book.current().is_none());
        assert!(book.answer(true).is_err());
    }

    #[test]
    fn an_outgoing_call_connects_to_the_device_that_accepted() {
        let mut book = CallBook::default();
        let bob = UserId::new();
        let call = book.start(bob, 0).unwrap();
        assert!(book.start(UserId::new(), 0).is_err());
        assert_eq!(book.on_signal(UserId::new(), OTHER, call.channel, CallSignal::Accept, 1), CallStep::Nothing);
        let CallStep::Connect(accepted) = book.on_signal(bob, OTHER, call.channel, CallSignal::Accept, 1) else { panic!() };
        assert_eq!(accepted.node.as_deref(), Some(OTHER));
        book.connected(call.channel, 2);
        assert_eq!(book.on_signal(bob, OTHER, call.channel, CallSignal::Accept, 3), CallStep::Nothing);
    }

    #[test]
    fn hang_ups_and_declines_end_only_the_matching_call() {
        let mut book = CallBook::default();
        let bob = UserId::new();
        let call = book.start(bob, 0).unwrap();
        assert_eq!(book.on_signal(bob, THEIRS, ChannelId::new(), CallSignal::HangUp, 1), CallStep::Nothing);
        assert!(matches!(book.on_signal(bob, THEIRS, call.channel, CallSignal::Decline, 1), CallStep::Ended { reason: "declined", .. }));

        let call = book.start(bob, 0).unwrap();
        assert!(matches!(book.on_signal(bob, THEIRS, call.channel, CallSignal::HangUp, 1), CallStep::Ended { reason: "missed", .. }));

        let call = book.start(bob, 0).unwrap();
        book.on_signal(bob, THEIRS, call.channel, CallSignal::Accept, 1);
        book.connected(call.channel, 1);
        assert!(matches!(book.on_signal(bob, THEIRS, call.channel, CallSignal::HangUp, 2), CallStep::Ended { reason: "hung_up", .. }));
    }

    #[test]
    fn a_call_answered_on_another_device_stops_ringing_here_only_while_ringing() {
        let mut book = CallBook::default();
        let alice = UserId::new();
        let channel = ringing(&mut book, alice);
        assert!(book.settled_elsewhere(alice, channel, CallSignal::HangUp).is_none());
        assert!(book.settled_elsewhere(alice, ChannelId::new(), CallSignal::Accept).is_none());
        assert!(book.settled_elsewhere(alice, channel, CallSignal::Accept).is_some());
        assert!(book.current().is_none());

        let channel = ringing(&mut book, alice);
        book.answer(true).unwrap();
        book.connected(channel, 1);
        assert!(book.settled_elsewhere(alice, channel, CallSignal::Accept).is_none());
        assert!(book.active().is_some());
    }

    #[test]
    fn ringing_runs_out_as_unanswered_or_missed() {
        let mut book = CallBook::default();
        let call = book.start(UserId::new(), 0).unwrap();
        assert_eq!(book.ring_out(ChannelId::new()), CallStep::Nothing);
        assert!(matches!(book.ring_out(call.channel), CallStep::Ended { reason: "unanswered", .. }));
        let channel = ringing(&mut book, UserId::new());
        assert!(matches!(book.ring_out(channel), CallStep::Ended { reason: "missed", .. }));
        let channel = ringing(&mut book, UserId::new());
        book.answer(true).unwrap();
        book.connected(channel, 1);
        assert_eq!(book.ring_out(channel), CallStep::Nothing);
    }

    #[test]
    fn exactly_one_side_dials() {
        assert!(should_dial("aa", "bb"));
        assert!(!should_dial("bb", "aa"));
        assert!(!should_dial("aa", "aa"));
    }

    #[test]
    fn the_view_counts_seconds_since_the_call_started() {
        let call = DmCall { partner: UserId::new(), channel: ChannelId::new(), node: None, outgoing: true, active: true, since_ms: 1_000 };
        let view = call.view("Bob".into(), call.state(), None, 4_500);
        assert_eq!((view.state.as_str(), view.seconds, view.name.as_str()), ("active", 3, "Bob"));
    }
}
