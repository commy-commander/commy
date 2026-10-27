use commy_core::tip_jar::clean_tip_jar;
use commy_core::{Permissions, ServerEvent, ServerState};

use crate::act::Act;
use crate::manage::need;

pub const TIP_JAR_INVALID: &str = "Use your Ko-fi page, like ko-fi.com/yourname";

pub fn set_tip_jar(state: &ServerState, perms: Permissions, page: Option<&str>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change the tip jar")?;
    let url = match page.map(str::trim).filter(|p| !p.is_empty()) {
        None => None,
        Some(page) => Some(clean_tip_jar(page).ok_or(TIP_JAR_INVALID)?),
    };
    Ok(ServerEvent::TipJarSet { server: state.server.id, url })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserId};

    #[test]
    fn owners_set_a_ko_fi_page_and_blank_clears_it() {
        let state = ServerState::new(Server::new("test", UserId::new()));
        let event = set_tip_jar(&state, Permissions::all(), Some(" ko-fi.com/rockets ")).unwrap();
        assert!(matches!(event, ServerEvent::TipJarSet { url: Some(ref u), .. } if u == "https://ko-fi.com/rockets"));
        assert!(matches!(set_tip_jar(&state, Permissions::all(), Some("  ")), Ok(ServerEvent::TipJarSet { url: None, .. })));
        assert_eq!(set_tip_jar(&state, Permissions::all(), Some("https://evil.example/x")).unwrap_err(), TIP_JAR_INVALID);
        assert!(set_tip_jar(&state, Permissions::default_member(), Some("rockets")).is_err());
    }
}
