use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedChannel {
    pub server: String,
    pub channel: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pins {
    #[serde(default)]
    pub dms: Vec<String>,
    #[serde(default)]
    pub channels: Vec<PinnedChannel>,
}

impl Pins {
    pub fn set_dm(&mut self, user: &str, pinned: bool) -> bool {
        let had = self.dms.iter().position(|u| u == user);
        match (had, pinned) {
            (None, true) => self.dms.push(user.to_string()),
            (Some(index), false) => {
                self.dms.remove(index);
            }
            _ => return false,
        }
        true
    }

    pub fn set_channel(&mut self, server: &str, channel: &str, pinned: bool) -> bool {
        let had = self.channels.iter().position(|c| c.server == server && c.channel == channel);
        match (had, pinned) {
            (None, true) => self.channels.push(PinnedChannel { server: server.to_string(), channel: channel.to_string() }),
            (Some(index), false) => {
                self.channels.remove(index);
            }
            _ => return false,
        }
        true
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_toggle_once_and_keep_their_order() {
        let mut pins = Pins::default();
        assert!(pins.set_dm("b", true));
        assert!(pins.set_dm("a", true));
        assert!(!pins.set_dm("a", true), "pinning twice changes nothing");
        assert_eq!(pins.dms, vec!["b".to_string(), "a".to_string()]);
        assert!(pins.set_dm("b", false));
        assert!(!pins.set_dm("b", false));
        assert_eq!(pins.dms, vec!["a".to_string()]);

        assert!(pins.set_channel("s1", "c1", true));
        assert!(pins.set_channel("s2", "c1", true));
        assert!(!pins.set_channel("s1", "c1", true));
        assert!(pins.set_channel("s1", "c1", false));
        assert_eq!(pins.channels, vec![PinnedChannel { server: "s2".into(), channel: "c1".into() }]);
    }
}
