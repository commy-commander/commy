use crate::ids::{RoleId, UserId};
use crate::server::Server;

pub const EVERYONE: &str = "everyone";
pub const HERE: &str = "here";

const ROLE_TOKEN_START: &str = "<@&";
const ROLE_TOKEN_END: char = '>';

pub fn role_token(role: RoleId) -> String {
    format!("{ROLE_TOKEN_START}{role}{ROLE_TOKEN_END}")
}

pub fn role_tokens(content: &str) -> Vec<RoleId> {
    let mut found = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find(ROLE_TOKEN_START) {
        let after = &rest[start + ROLE_TOKEN_START.len()..];
        let Some(end) = after.find(ROLE_TOKEN_END) else {
            break;
        };
        if let Ok(role) = after[..end].parse::<RoleId>() {
            if !found.contains(&role) {
                found.push(role);
            }
        }
        rest = &after[end + ROLE_TOKEN_END.len_utf8()..];
    }
    found
}

pub fn names_mentioned(content: &str, names: &[&str]) -> bool {
    let lower = content.to_lowercase();
    names
        .iter()
        .map(|name| name.to_lowercase())
        .filter(|name| !name.is_empty())
        .any(|name| name_mentioned(&lower, &name))
}

fn name_mentioned(lower: &str, name: &str) -> bool {
    let needle = format!("@{name}");
    lower.match_indices(&needle).any(|(at, _)| {
        lower[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

pub fn mentions_everyone(content: &str) -> bool {
    names_mentioned(content, &[EVERYONE, HERE])
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mentions {
    pub name: bool,
    pub role: bool,
    pub everyone: bool,
}

impl Mentions {
    pub fn by_name(name: bool) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }

    pub fn any(self) -> bool {
        self.name || self.role || self.everyone
    }

    pub fn merge(&mut self, other: Mentions) {
        self.name |= other.name;
        self.role |= other.role;
        self.everyone |= other.everyone;
    }

    pub fn kinds(self) -> Vec<&'static str> {
        [
            (self.name, "name"),
            (self.role, "role"),
            (self.everyone, "everyone"),
        ]
        .into_iter()
        .filter_map(|(on, kind)| on.then_some(kind))
        .collect()
    }
}

impl Server {
    pub fn can_mention_everyone(&self, user: UserId) -> bool {
        self.permissions_for(user).can_mention_everyone()
    }

    pub fn can_mention_role(&self, user: UserId, role: RoleId) -> bool {
        self.roles.iter().any(|r| r.id == role && r.mentionable) || self.can_mention_everyone(user)
    }

    pub fn mentions_of(
        &self,
        author: UserId,
        content: &str,
        viewer: UserId,
        names: &[&str],
    ) -> Mentions {
        let role = self.member(viewer).is_some_and(|member| {
            role_tokens(content)
                .into_iter()
                .any(|role| member.roles.contains(&role) && self.can_mention_role(author, role))
        });
        Mentions {
            name: names_mentioned(content, names),
            role,
            everyone: mentions_everyone(content) && self.can_mention_everyone(author),
        }
    }

    pub fn mentioned_by(
        &self,
        author: UserId,
        content: &str,
        viewer: UserId,
        names: &[&str],
    ) -> bool {
        self.mentions_of(author, content, viewer, names).any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::permissions::Permissions;
    use crate::server::Role;
    use crate::test_support::{fixture, join};
    use crate::ServerState;

    fn add_role(
        state: &mut ServerState,
        name: &str,
        permissions: Permissions,
        mentionable: bool,
    ) -> RoleId {
        let role = Role {
            id: RoleId::new(),
            name: name.into(),
            color: None,
            permissions,
            position: state.server.roles.len() as u32,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
            mentionable,
        };
        let id = role.id;
        state
            .apply(ServerEvent::RoleCreate {
                server: state.server.id,
                role,
            })
            .unwrap();
        id
    }

    fn give(state: &mut ServerState, user: UserId, role: RoleId) {
        state
            .apply(ServerEvent::RoleAssign {
                server: state.server.id,
                user,
                role,
            })
            .unwrap();
    }

    #[test]
    fn role_tokens_parse_once_each_and_skip_junk() {
        let (a, b) = (RoleId::new(), RoleId::new());
        let text = format!(
            "ping {} and {} again {} <@&nope> <@&",
            role_token(a),
            role_token(b),
            role_token(a)
        );
        assert_eq!(role_tokens(&text), vec![a, b]);
        assert!(role_tokens("no tokens here <@everyone>").is_empty());
        assert_eq!(role_token(a), format!("<@&{a}>"));
    }

    #[test]
    fn names_match_whole_words_case_insensitively() {
        assert!(names_mentioned("hey @sam, lunch?", &["sam"]));
        assert!(names_mentioned("HEY @Sam", &["sam"]));
        assert!(!names_mentioned("hey @samantha", &["sam"]));
        assert!(!names_mentioned("mail sam@example.org", &["sam"]));
        assert!(!names_mentioned("@", &[""]));
        assert!(mentions_everyone("@everyone look"));
        assert!(mentions_everyone("ping @HERE"));
        assert!(!mentions_everyone("@everyone_else"));
    }

    #[test]
    fn mentions_say_whether_they_came_by_name_role_or_everyone() {
        let (mut state, _, owner) = fixture();
        let viewer = join(&mut state, "viewer");
        let crew = add_role(&mut state, "crew", Permissions::NONE, true);
        give(&mut state, viewer, crew);
        let of = |content: &str| state.server.mentions_of(owner, content, viewer, &["viewer"]);
        assert_eq!(of("@viewer hi"), Mentions::by_name(true));
        assert_eq!(of("hello"), Mentions::default());
        let both = of(&format!("@here {}", role_token(crew)));
        assert!(both.role && both.everyone && !both.name);
        assert_eq!(both.kinds(), vec!["role", "everyone"]);
        let mut seen = Mentions::by_name(true);
        seen.merge(of("@everyone"));
        assert_eq!(seen.kinds(), vec!["name", "everyone"]);
        assert!(!Mentions::default().any());
    }

    #[test]
    fn everyone_and_here_only_count_from_senders_with_the_permission() {
        let (mut state, _, owner) = fixture();
        let plain = join(&mut state, "plain");
        let viewer = join(&mut state, "viewer");
        assert!(state.server.can_mention_everyone(owner));
        assert!(!state.server.can_mention_everyone(plain));
        assert!(state
            .server
            .mentioned_by(owner, "@everyone hi", viewer, &["viewer"]));
        assert!(state
            .server
            .mentioned_by(owner, "@here hi", viewer, &["viewer"]));
        assert!(!state
            .server
            .mentioned_by(plain, "@everyone hi", viewer, &["viewer"]));
        assert!(state
            .server
            .mentioned_by(plain, "@viewer hi", viewer, &["viewer"]));
        let herald = add_role(&mut state, "herald", Permissions::MENTION_EVERYONE, false);
        give(&mut state, plain, herald);
        assert!(state
            .server
            .mentioned_by(plain, "@everyone hi", viewer, &["viewer"]));
    }

    #[test]
    fn role_mentions_reach_holders_when_the_role_is_open_or_the_sender_may_ping_everyone() {
        let (mut state, _, owner) = fixture();
        let plain = join(&mut state, "plain");
        let viewer = join(&mut state, "viewer");
        let closed = add_role(&mut state, "closed", Permissions::NONE, false);
        let open = add_role(&mut state, "open", Permissions::NONE, true);
        give(&mut state, viewer, closed);
        give(&mut state, viewer, open);
        let ping_closed = format!("hey {}", role_token(closed));
        let ping_open = format!("hey {}", role_token(open));
        assert!(state.server.mentioned_by(owner, &ping_closed, viewer, &[]));
        assert!(!state.server.mentioned_by(plain, &ping_closed, viewer, &[]));
        assert!(state.server.mentioned_by(plain, &ping_open, viewer, &[]));
        assert!(
            !state.server.mentioned_by(owner, &ping_open, plain, &[]),
            "plain does not hold the role"
        );
        assert!(
            !state
                .server
                .mentioned_by(owner, &ping_open, UserId::new(), &[]),
            "strangers hold nothing"
        );
    }
}
