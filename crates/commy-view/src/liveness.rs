use std::collections::{HashMap, HashSet};
use std::time::Duration;

use commy_core::{action, devices, Action, Presence, ServerEvent, ServerId, ServerState, UserId};
use web_time::Instant;

pub const BEAT: Duration = Duration::from_secs(45);

pub const EXPIRE: Duration = Duration::from_secs(120);

pub const SERVICE_BEAT: Duration = Duration::from_secs(5 * 60);

pub const SERVICE_EXPIRE: Duration = Duration::from_secs(11 * 60);

pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

type DeviceSaid = HashMap<String, (Presence, Instant, i64)>;

#[derive(Debug, Default)]
pub struct Liveness {
    heard: HashMap<(ServerId, UserId), Instant>,
    lasting: HashMap<(ServerId, UserId), Instant>,
    devices: HashMap<(ServerId, UserId), DeviceSaid>,
    device_heard: HashMap<(ServerId, UserId, String), Instant>,
    last_sweep_ms: Option<i64>,
}

pub fn speaker(event: &ServerEvent, signed_by: Option<&Action>) -> Option<UserId> {
    if action::relayable(event) {
        return None;
    }
    let actor = action::actor(event)?;
    signed_by.is_none_or(|a| a.by == actor).then_some(actor)
}

pub fn node_hex(node: &[u8; 32]) -> String {
    data_encoding::HEXLOWER.encode(node)
}

impl Liveness {
    pub fn heard(&mut self, server: ServerId, user: UserId, at: Instant) {
        self.heard.insert((server, user), at);
    }

    pub fn heard_through_service(&mut self, server: ServerId, user: UserId, now: Instant) {
        self.heard.insert((server, user), now);
        self.lasting.insert((server, user), now + SERVICE_EXPIRE);
    }

    pub fn heard_device(&mut self, server: ServerId, user: UserId, node: &str, now: Instant) {
        self.device_heard.insert((server, user, node.to_ascii_lowercase()), now);
    }

    pub fn settle_device_presence(
        &mut self,
        state: &mut ServerState,
        user: UserId,
        sender: &[u8; 32],
        presence: Presence,
        now: Instant,
        said_at: Option<i64>,
    ) -> bool {
        let node = node_hex(sender);
        if !state.devices_of(user).iter().any(|d| d.node == node) {
            return false;
        }
        let said = self.devices.entry((state.server.id, user)).or_default();
        let said_at = said_at.unwrap_or(i64::MIN);
        let newest = said.get(&node).is_none_or(|(_, _, before)| said_at >= *before);
        let voice_device_left = newest && presence == Presence::Offline && state.voice_device(user) == Some(node.as_str());
        let left_voice = voice_device_left && state.end_voice(user);
        if newest {
            said.insert(node, (presence, now, said_at));
        }
        said.retain(|_, (_, at, _)| now.duration_since(*at) <= SERVICE_EXPIRE);
        if said.len() < 2 {
            return left_voice;
        }
        let shown = devices::combined_presence(said.values().map(|(p, _, _)| *p));
        match state.profiles.get_mut(&user) {
            Some(profile) if profile.presence != shown => {
                profile.presence = shown;
                true
            }
            _ => left_voice,
        }
    }

    pub fn sweep<'a>(
        &mut self,
        servers: impl IntoIterator<Item = (&'a ServerId, &'a mut ServerState)>,
        me: UserId,
        now: Instant,
        wall_ms: i64,
        expire: Duration,
    ) -> Vec<ServerId> {
        let asleep = self.last_sweep_ms.is_some_and(|last| wall_ms.saturating_sub(last) > expire.as_millis() as i64);
        self.last_sweep_ms = Some(wall_ms);
        if asleep {
            self.heard.clear();
            self.lasting.clear();
            self.device_heard.clear();
        }
        self.lasting.retain(|_, until| now < *until);
        let mut changed = Vec::new();
        let mut alive = HashSet::new();
        for (sid, state) in servers {
            let mut watched: Vec<UserId> = state
                .profiles
                .values()
                .filter(|p| p.presence.is_visible_online())
                .map(|p| p.id)
                .chain(state.voice_states.keys().copied())
                .filter(|user| *user != me)
                .collect();
            watched.sort();
            watched.dedup();
            let mut gone = false;
            for user in watched {
                let last = *self.heard.entry((*sid, user)).or_insert(now);
                let lasting = self.lasting.contains_key(&(*sid, user));
                if now.duration_since(last) > expire && !lasting {
                    gone |= state.apply(ServerEvent::PresenceUpdate { user, presence: Presence::Offline }) == Ok(true);
                    gone |= state.end_voice(user);
                    continue;
                }
                alive.insert((*sid, user));
                if self.voice_device_went_quiet(state, user, now, expire, lasting) {
                    gone |= state.end_voice(user);
                }
            }
            if gone {
                changed.push(*sid);
            }
        }
        self.heard.retain(|key, _| alive.contains(key));
        self.devices.retain(|key, _| alive.contains(key));
        self.device_heard.retain(|(sid, user, _), _| alive.contains(&(*sid, *user)));
        changed
    }

    fn voice_device_went_quiet(&mut self, state: &ServerState, user: UserId, now: Instant, expire: Duration, lasting: bool) -> bool {
        let Some(node) = state.voice_device(user) else { return false };
        let last = *self.device_heard.entry((state.server.id, user, node.to_ascii_lowercase())).or_insert(now);
        let window = if lasting { expire.max(SERVICE_EXPIRE) } else { expire };
        now.duration_since(last) > window
    }

    pub fn rest(&mut self) {
        self.heard.clear();
        self.lasting.clear();
        self.devices.clear();
        self.device_heard.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{ChannelId, Server, UserProfile};

    fn server_with(members: &[&UserProfile]) -> HashMap<ServerId, ServerState> {
        let server = Server::new("Moonbase", members[0].id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        for profile in members {
            state.apply(ServerEvent::MemberJoin { server: sid, profile: (*profile).clone() }).unwrap();
        }
        HashMap::from([(sid, state)])
    }

    #[test]
    fn a_member_heard_only_through_the_service_goes_offline_after_its_window() {
        let me = UserProfile::new("me");
        let friend = UserProfile::new("friend");
        let mut servers = server_with(&[&me, &friend]);
        let sid = *servers.keys().next().unwrap();
        let mut live = Liveness::default();
        let start = Instant::now();
        live.heard_through_service(sid, friend.id, start);
        assert!(live.sweep(servers.iter_mut(), me.id, start + SERVICE_BEAT, 0, EXPIRE).is_empty());
        let later = start + SERVICE_EXPIRE + SWEEP_EVERY;
        assert_eq!(live.sweep(servers.iter_mut(), me.id, later, 15_000, EXPIRE), vec![sid]);
        assert_eq!(servers[&sid].profiles[&friend.id].presence, Presence::Offline);
        assert_eq!(servers[&sid].profiles[&me.id].presence, Presence::Online);
    }

    struct Room {
        servers: HashMap<ServerId, ServerState>,
        sid: ServerId,
        me: UserId,
        friend: UserId,
        voice: ChannelId,
    }

    const DESKTOP: [u8; 32] = [1; 32];
    const PHONE: [u8; 32] = [2; 32];

    fn room() -> Room {
        let me = UserProfile::new("me");
        let friend = UserProfile::new("friend");
        let servers = server_with(&[&me, &friend]);
        let sid = *servers.keys().next().unwrap();
        let voice = servers[&sid].server.channels[1].id;
        Room { servers, sid, me: me.id, friend: friend.id, voice }
    }

    impl Room {
        fn state(&mut self) -> &mut ServerState {
            self.servers.get_mut(&self.sid).unwrap()
        }

        fn announce(&mut self, node: [u8; 32], voice: Option<ChannelId>, since: i64) {
            let (sid, friend) = (self.sid, self.friend);
            let event = ServerEvent::DeviceAnnounce { server: sid, user: friend, node: node_hex(&node), voice, since: Some(since) };
            self.state().apply(event).unwrap();
        }

        fn seated(&self) -> Option<ChannelId> {
            self.servers[&self.sid].voice_states.get(&self.friend).copied()
        }

        fn sweep(&mut self, live: &mut Liveness, at: Instant, wall_ms: i64) -> Vec<ServerId> {
            live.sweep(self.servers.iter_mut(), self.me, at, wall_ms, EXPIRE)
        }
    }

    #[test]
    fn a_seat_whose_device_goes_quiet_is_emptied_while_another_device_keeps_the_person_online() {
        let mut room = room();
        let mut live = Liveness::default();
        let (sid, friend, voice) = (room.sid, room.friend, room.voice);
        let start = Instant::now();
        room.announce(PHONE, None, 0);
        room.announce(DESKTOP, Some(voice), 1_000);
        live.heard_device(sid, friend, &node_hex(&DESKTOP), start);
        live.heard(sid, friend, start);
        assert!(room.sweep(&mut live, start, 0).is_empty());

        let later = start + EXPIRE + SWEEP_EVERY;
        live.heard(sid, friend, later);
        live.heard_device(sid, friend, &node_hex(&PHONE), later);
        assert_eq!(room.sweep(&mut live, later, 15_000), vec![sid]);
        assert_eq!(room.seated(), None, "the desktop holding the seat went quiet");
        assert_eq!(room.servers[&sid].profiles[&friend].presence, Presence::Online, "the phone still speaks for them");
    }

    #[test]
    fn a_seat_stays_while_its_device_keeps_beating() {
        let mut room = room();
        let mut live = Liveness::default();
        let (sid, friend, voice) = (room.sid, room.friend, room.voice);
        let start = Instant::now();
        room.announce(DESKTOP, Some(voice), 1_000);
        let mut at = start;
        for beat in 0..6 {
            live.heard(sid, friend, at);
            live.heard_device(sid, friend, &node_hex(&DESKTOP), at);
            assert!(room.sweep(&mut live, at, beat * 15_000).is_empty());
            at += BEAT;
        }
        assert_eq!(room.seated(), Some(voice));
    }

    #[test]
    fn someone_appearing_offline_in_voice_leaves_it_when_nobody_hears_from_them() {
        let mut room = room();
        let mut live = Liveness::default();
        let (sid, friend, voice) = (room.sid, room.friend, room.voice);
        room.state().apply(ServerEvent::PresenceUpdate { user: friend, presence: Presence::Invisible }).unwrap();
        room.state().apply(ServerEvent::VoiceState { user: friend, channel: Some(voice), muted: false, deafened: false, since: None }).unwrap();
        let start = Instant::now();
        assert!(room.sweep(&mut live, start, 0).is_empty());
        let later = start + EXPIRE + SWEEP_EVERY;
        assert_eq!(room.sweep(&mut live, later, 15_000), vec![sid]);
        assert_eq!(room.seated(), None);
    }

    #[test]
    fn a_seat_heard_only_through_the_service_lasts_its_longer_window() {
        let mut room = room();
        let mut live = Liveness::default();
        let (sid, friend, voice) = (room.sid, room.friend, room.voice);
        let start = Instant::now();
        room.announce(DESKTOP, Some(voice), 1_000);
        live.heard_through_service(sid, friend, start);
        live.heard_device(sid, friend, &node_hex(&DESKTOP), start);
        assert!(room.sweep(&mut live, start + SERVICE_BEAT, 15_000).is_empty());
        assert_eq!(room.seated(), Some(voice));
        assert_eq!(room.sweep(&mut live, start + SERVICE_EXPIRE + SWEEP_EVERY, 30_000), vec![sid]);
        assert_eq!(room.seated(), None);
    }

    #[test]
    fn going_offline_on_the_device_in_voice_empties_the_seat_and_a_stale_goodbye_does_not() {
        let mut room = room();
        let mut live = Liveness::default();
        let (sid, friend, voice) = (room.sid, room.friend, room.voice);
        let now = Instant::now();
        room.announce(PHONE, None, 0);
        room.announce(DESKTOP, Some(voice), 1_000);

        let state = room.servers.get_mut(&sid).unwrap();
        live.settle_device_presence(state, friend, &PHONE, Presence::Offline, now, Some(10));
        assert_eq!(room.seated(), Some(voice), "the phone quitting says nothing about voice");

        let state = room.servers.get_mut(&sid).unwrap();
        live.settle_device_presence(state, friend, &DESKTOP, Presence::Online, now, Some(50));
        let state = room.servers.get_mut(&sid).unwrap();
        live.settle_device_presence(state, friend, &DESKTOP, Presence::Offline, now, Some(20));
        assert_eq!(room.seated(), Some(voice), "a goodbye left earlier, from before the desktop came back");
        let state = room.servers.get_mut(&sid).unwrap();
        live.settle_device_presence(state, friend, &DESKTOP, Presence::Invisible, now, Some(60));
        assert_eq!(room.seated(), Some(voice), "appearing offline is not leaving");
        let state = room.servers.get_mut(&sid).unwrap();
        assert!(live.settle_device_presence(state, friend, &DESKTOP, Presence::Offline, now, Some(70)));
        assert_eq!(room.seated(), None);
        assert_eq!(room.servers[&sid].voice_device(friend), None);
    }

    #[test]
    fn waking_from_sleep_gives_seats_a_fresh_window() {
        let mut room = room();
        let mut live = Liveness::default();
        let (sid, friend, voice) = (room.sid, room.friend, room.voice);
        let start = Instant::now();
        room.announce(DESKTOP, Some(voice), 1_000);
        live.heard(sid, friend, start);
        live.heard_device(sid, friend, &node_hex(&DESKTOP), start);
        room.sweep(&mut live, start, 0);
        let after_sleep = start + Duration::from_secs(3600);
        assert!(room.sweep(&mut live, after_sleep, 3_600_000).is_empty());
        assert_eq!(room.seated(), Some(voice));
    }
}
