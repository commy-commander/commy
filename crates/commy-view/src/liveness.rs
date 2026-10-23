use std::collections::{HashMap, HashSet};
use std::time::Duration;

use commy_core::{action, devices, Action, Presence, ServerEvent, ServerId, ServerState, UserId};
use web_time::Instant;

pub const BEAT: Duration = Duration::from_secs(45);

pub const EXPIRE: Duration = Duration::from_secs(120);

pub const SERVICE_BEAT: Duration = Duration::from_secs(5 * 60);

pub const SERVICE_EXPIRE: Duration = Duration::from_secs(11 * 60);

pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

type DeviceSaid = HashMap<String, (Presence, Instant)>;

#[derive(Debug, Default)]
pub struct Liveness {
    heard: HashMap<(ServerId, UserId), Instant>,
    lasting: HashMap<(ServerId, UserId), Instant>,
    devices: HashMap<(ServerId, UserId), DeviceSaid>,
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

    pub fn settle_device_presence(
        &mut self,
        state: &mut ServerState,
        user: UserId,
        sender: &[u8; 32],
        presence: Presence,
        now: Instant,
    ) -> bool {
        let node = node_hex(sender);
        if !state.devices_of(user).iter().any(|d| d.node == node) {
            return false;
        }
        let said = self.devices.entry((state.server.id, user)).or_default();
        said.insert(node, (presence, now));
        said.retain(|_, (_, at)| now.duration_since(*at) <= SERVICE_EXPIRE);
        if said.len() < 2 {
            return false;
        }
        let shown = devices::combined_presence(said.values().map(|(p, _)| *p));
        match state.profiles.get_mut(&user) {
            Some(profile) if profile.presence != shown => {
                profile.presence = shown;
                true
            }
            _ => false,
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
        }
        self.lasting.retain(|_, until| now < *until);
        let mut changed = Vec::new();
        let mut online = HashSet::new();
        for (sid, state) in servers {
            let shown: Vec<UserId> = state
                .profiles
                .values()
                .filter(|p| p.id != me && p.presence.is_visible_online())
                .map(|p| p.id)
                .collect();
            let mut gone = false;
            for user in shown {
                let last = *self.heard.entry((*sid, user)).or_insert(now);
                let lasting = self.lasting.contains_key(&(*sid, user));
                if now.duration_since(last) > expire && !lasting {
                    gone |= state.apply(ServerEvent::PresenceUpdate { user, presence: Presence::Offline }) == Ok(true);
                } else {
                    online.insert((*sid, user));
                }
            }
            if gone {
                changed.push(*sid);
            }
        }
        self.heard.retain(|key, _| online.contains(key));
        self.devices.retain(|key, _| online.contains(key));
        changed
    }

    pub fn rest(&mut self) {
        self.heard.clear();
        self.lasting.clear();
        self.devices.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

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
}
