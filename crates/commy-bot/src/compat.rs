use chrono::DateTime;
use commy_core::server::{emoji_token, parse_emoji_token};
use commy_core::{ChannelId, ChannelKind, RoleId, ServerState, UserId};
use serde::Deserialize;

pub const MAX_EMBEDS: usize = 10;
pub const MAX_FIELDS: usize = 25;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Embed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub author: Option<Named>,
    pub fields: Vec<Field>,
    pub image: Option<Linked>,
    pub thumbnail: Option<Linked>,
    pub footer: Option<Footer>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Linked {
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Footer {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Field {
    pub name: String,
    pub value: String,
}

pub fn compose(content: Option<&str>, embeds: &[Embed], state: &ServerState) -> String {
    let mut blocks: Vec<String> = content.map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).into_iter().collect();
    blocks.extend(embeds.iter().take(MAX_EMBEDS).map(flatten).filter(|b| !b.is_empty()));
    to_commy(&blocks.join("\n\n"), state)
}

pub fn flatten(embed: &Embed) -> String {
    let link = |l: &Option<Linked>| l.as_ref().map(|l| l.url.trim().to_string());
    let mut lines: Vec<String> = Vec::new();
    lines.extend(embed.author.as_ref().map(|a| a.name.clone()));
    lines.extend(embed.title.clone());
    lines.extend(embed.description.clone());
    for field in embed.fields.iter().take(MAX_FIELDS) {
        lines.push(match (field.name.trim(), field.value.trim()) {
            ("", value) => value.to_string(),
            (name, "") => name.to_string(),
            (name, value) if value.contains('\n') => format!("{name}:\n{value}"),
            (name, value) => format!("{name}: {value}"),
        });
    }
    lines.extend(embed.url.clone());
    lines.extend(link(&embed.image).or_else(|| link(&embed.thumbnail)));
    lines.extend(embed.footer.as_ref().map(|f| f.text.clone()));
    lines.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

pub fn to_commy(text: &str, state: &ServerState) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            out.push_str(tail);
            return out;
        };
        let token = &tail[..=end];
        match translate(token, state) {
            Some(plain) => out.push_str(&plain),
            None => out.push_str(token),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn translate(token: &str, state: &ServerState) -> Option<String> {
    let inner = token.strip_prefix('<')?.strip_suffix('>')?;
    if let Some(user) = inner.strip_prefix("@!").or_else(|| inner.strip_prefix('@')) {
        let user: UserId = user.parse().ok()?;
        return Some(format!("@{}", state.profiles.get(&user)?.visible_name()));
    }
    if let Some(channel) = inner.strip_prefix('#') {
        let channel: ChannelId = channel.parse().ok()?;
        return Some(format!("#{}", state.server.channel(channel)?.name));
    }
    if let Some(stamp) = inner.strip_prefix("t:") {
        let seconds: i64 = stamp.split(':').next()?.parse().ok()?;
        return Some(DateTime::from_timestamp(seconds, 0)?.format("%Y-%m-%d %H:%M UTC").to_string());
    }
    if parse_emoji_token(token).is_some() {
        return None;
    }
    let named = inner.strip_prefix("a:").or_else(|| inner.strip_prefix(':'))?;
    let (name, _) = named.split_once(':')?;
    let own = state.server.emojis.iter().find(|e| e.name == name).map(|e| emoji_token(&e.name, &e.blob));
    Some(own.filter(|t| parse_emoji_token(t).is_some()).unwrap_or_else(|| format!(":{name}:")))
}

pub fn mentions(content: &str, state: &ServerState) -> Vec<UserId> {
    let lower = content.to_lowercase();
    let mut found: Vec<UserId> = state
        .server
        .members
        .iter()
        .filter(|m| state.server.is_current_member(m.user))
        .filter_map(|m| state.profiles.get(&m.user))
        .filter(|p| [p.visible_name(), p.username.as_str()].iter().any(|name| mentioned(&lower, name)))
        .map(|p| p.id)
        .collect();
    found.dedup();
    found
}

fn mentioned(lower: &str, name: &str) -> bool {
    let needle = format!("@{}", name.to_lowercase());
    lower.match_indices(&needle).any(|(at, _)| {
        lower[at + needle.len()..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

pub fn channel(state: &ServerState, text: &str) -> Option<ChannelId> {
    let text = text.trim();
    let bare = text.strip_prefix("<#").and_then(|t| t.strip_suffix('>')).unwrap_or(text);
    if let Ok(id) = bare.parse::<ChannelId>() {
        return state.server.channel(id).map(|c| c.id);
    }
    let name = bare.trim_start_matches('#');
    state
        .server
        .channels
        .iter()
        .filter(|c| matches!(c.kind, ChannelKind::Text | ChannelKind::Announcement))
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .map(|c| c.id)
}

pub fn role(state: &ServerState, text: &str) -> Option<RoleId> {
    let text = text.trim();
    let bare = text.strip_prefix("<@&").and_then(|t| t.strip_suffix('>')).unwrap_or(text);
    let name = bare.trim_start_matches('@');
    state.server.roles.iter().find(|r| r.id.to_string() == bare || r.name.eq_ignore_ascii_case(name)).map(|r| r.id)
}

pub fn member(state: &ServerState, text: &str) -> Option<UserId> {
    let text = text.trim();
    let bare = text.strip_prefix("<@").and_then(|t| t.strip_suffix('>')).map(|t| t.trim_start_matches('!')).unwrap_or(text);
    if let Ok(id) = bare.parse::<UserId>() {
        return state.server.is_current_member(id).then_some(id);
    }
    let name = bare.trim_start_matches('@');
    state
        .server
        .members
        .iter()
        .filter_map(|m| state.profiles.get(&m.user))
        .find(|p| p.visible_name().eq_ignore_ascii_case(name) || p.username.eq_ignore_ascii_case(name))
        .map(|p| p.id)
        .filter(|id| state.server.is_current_member(*id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::test_support::fixture;
    use commy_core::{Permissions, Role, ServerEvent};

    fn with_role(state: &mut ServerState) -> RoleId {
        let role = Role {
            id: RoleId::new(),
            name: "Helpers".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let id = role.id;
        state.apply(ServerEvent::RoleCreate { server: state.server.id, role }).unwrap();
        id
    }

    #[test]
    fn mention_tokens_become_names() {
        let (mut state, channel, owner) = fixture();
        let role = with_role(&mut state);
        let name = state.profiles[&owner].visible_name().to_string();
        let channel_name = state.server.channel(channel).unwrap().name.clone();
        let text = format!("hi <@{owner}> and <@!{owner}>, ask <@&{role}> in <#{channel}> by <t:1792500000:R>");
        assert_eq!(
            to_commy(&text, &state),
            format!("hi @{name} and @{name}, ask <@&{role}> in #{channel_name} by 2026-10-20 12:40 UTC"),
            "role tokens stay as role mentions"
        );
    }

    #[test]
    fn unknown_tokens_and_plain_angles_stay() {
        let (state, ..) = fixture();
        let unknown = format!("<@{}> 1 < 2 > 0 <b>", UserId::new());
        assert_eq!(to_commy(&unknown, &state), unknown);
        assert_eq!(to_commy("a < b", &state), "a < b");
        assert_eq!(to_commy("party <a:dance:123456> <:wave:42>", &state), "party :dance: :wave:");
    }

    #[test]
    fn embeds_flatten_to_plain_lines() {
        let (state, ..) = fixture();
        let embed: Embed = serde_json::from_value(serde_json::json!({
            "author": {"name": "Build bot"},
            "title": "Build passed",
            "description": "main is green",
            "url": "https://example.com/run/1",
            "color": 3066993,
            "fields": [{"name": "Branch", "value": "main", "inline": true}, {"name": "Time", "value": "3m"}],
            "image": {"url": "https://example.com/badge.png"},
            "footer": {"text": "ci"}
        }))
        .unwrap();
        let text = compose(Some("Heads up"), &[embed, Embed::default()], &state);
        assert_eq!(
            text,
            "Heads up\n\nBuild bot\nBuild passed\nmain is green\nBranch: main\nTime: 3m\nhttps://example.com/run/1\nhttps://example.com/badge.png\nci"
        );
        assert_eq!(compose(None, &[], &state), "");
    }

    #[test]
    fn names_resolve_to_ids() {
        let (mut state, channel, owner) = fixture();
        let role = with_role(&mut state);
        let channel_name = state.server.channel(channel).unwrap().name.clone();
        let name = state.profiles[&owner].visible_name().to_string();
        assert_eq!(self::channel(&state, &format!("#{}", channel_name.to_uppercase())), Some(channel));
        assert_eq!(self::channel(&state, &format!("<#{channel}>")), Some(channel));
        assert_eq!(self::channel(&state, "nowhere"), None);
        assert_eq!(self::role(&state, "@helpers"), Some(role));
        assert_eq!(self::role(&state, &format!("<@&{role}>")), Some(role));
        assert_eq!(member(&state, &format!("<@!{owner}>")), Some(owner));
        assert_eq!(member(&state, &format!("@{name}")), Some(owner));
        assert_eq!(member(&state, &UserId::new().to_string()), None);
        assert_eq!(mentions(&format!("thanks @{name}!"), &state), vec![owner]);
        assert!(mentions(&format!("mail @{name}x"), &state).is_empty());
    }
}
