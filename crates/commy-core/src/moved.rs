use crate::identity::{Identity, IdentityKey, Signature};
use crate::ids::UserId;
use serde::{Deserialize, Serialize};

const MOVE_PREFIX: &str = "commy-moved/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovePointer {
    pub from: String,
    pub to: String,
    pub user: UserId,
    pub identity: IdentityKey,
    pub at: i64,
    pub proof: Signature,
}

pub fn move_message(from: &str, to: &str, user: UserId, identity: &IdentityKey, at: i64) -> String {
    format!("{MOVE_PREFIX}\n{from}\n{to}\n{user}\n{}\n{at}", identity.to_hex())
}

impl MovePointer {
    pub fn sign(identity: &Identity, from: &str, to: &str, user: UserId, at: i64) -> MovePointer {
        let public = identity.public();
        let proof = identity.sign(move_message(from, to, user, &public, at).as_bytes());
        MovePointer { from: from.to_string(), to: to.to_string(), user, identity: public, at, proof }
    }

    pub fn holds_together(&self) -> bool {
        crate::sync_host::is_sync_host(&self.from)
            && crate::sync_host::is_sync_host(&self.to)
            && self.from != self.to
            && self.at > 0
            && self.identity.verify(move_message(&self.from, &self.to, self.user, &self.identity, self.at).as_bytes(), &self.proof)
    }

    pub fn moves(&self, user: UserId, identity: &IdentityKey, from: &str) -> bool {
        self.holds_together() && self.user == user && &self.identity == identity && self.from == from
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: i64 = 1_800_000_000_000;

    #[test]
    fn only_the_identity_itself_can_say_where_it_moved() {
        let me = Identity::from_bytes(&[3; 32]);
        let user = UserId::new();
        let pointer = MovePointer::sign(&me, "https://commy.chat", "https://friends.org", user, AT);
        assert!(pointer.holds_together());
        assert!(pointer.moves(user, &me.public(), "https://commy.chat"));
        assert!(!pointer.moves(UserId::new(), &me.public(), "https://commy.chat"));
        assert!(!pointer.moves(user, &me.public(), "https://friends.org"));
        let mut redirected = pointer.clone();
        redirected.to = "https://evil.org".into();
        assert!(!redirected.holds_together(), "the home cannot change where the pointer leads");
        let stranger = Identity::from_bytes(&[4; 32]);
        let mut claimed = pointer.clone();
        claimed.identity = stranger.public();
        assert!(!claimed.holds_together());
        let nowhere = MovePointer::sign(&me, "https://commy.chat", "https://commy.chat", user, AT);
        assert!(!nowhere.holds_together(), "a move must go somewhere else");
        let loose = MovePointer::sign(&me, "https://commy.chat", "friends.org", user, AT);
        assert!(!loose.holds_together(), "homes are written as clean https origins");
    }

    #[test]
    fn the_signed_words_match_what_the_account_server_checks() {
        let me = Identity::from_bytes(&[5; 32]);
        let user: UserId = "6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e".parse().unwrap();
        let text = move_message("https://commy.chat", "https://friends.org", user, &me.public(), 7);
        assert_eq!(text, format!("commy-moved/v1\nhttps://commy.chat\nhttps://friends.org\n{user}\n{}\n7", me.public().to_hex()));
        let json = serde_json::to_value(MovePointer::sign(&me, "https://commy.chat", "https://friends.org", user, 7)).unwrap();
        assert_eq!(json["identity"], me.public().to_hex());
        assert!(json["proof"].as_str().is_some_and(|proof| proof.len() == 128));
    }
}
