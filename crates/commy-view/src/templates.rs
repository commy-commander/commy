use commy_core::server::ChannelKind;
use commy_core::{BuiltinTemplate, ServerTemplate};
use serde::Serialize;

use crate::act::Act;

#[derive(Debug, Serialize)]
pub struct TemplateChannelView {
    pub name: String,
    pub kind: ChannelKind,
    pub category: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TemplateRoleView {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TemplateView {
    pub id: Option<String>,
    pub title: String,
    pub blurb: String,
    pub name: String,
    pub channels: Vec<TemplateChannelView>,
    pub roles: Vec<TemplateRoleView>,
    pub has_rules: bool,
}

impl TemplateView {
    fn of(template: &ServerTemplate, builtin: Option<BuiltinTemplate>) -> Self {
        Self {
            id: builtin.map(|b| b.id().to_string()),
            title: builtin.map_or_else(|| template.name.clone(), |b| b.title().to_string()),
            blurb: builtin.map(|b| b.blurb().to_string()).unwrap_or_default(),
            name: template.name.clone(),
            channels: template
                .channels
                .iter()
                .map(|c| TemplateChannelView { name: c.name.clone(), kind: c.kind, category: c.category })
                .collect(),
            roles: template
                .roles
                .iter()
                .map(|r| TemplateRoleView { name: r.name.clone(), color: r.color.clone() })
                .collect(),
            has_rules: template.rules.is_some(),
        }
    }
}

pub fn resolve(choice: Option<&str>) -> Act<ServerTemplate> {
    match choice.map(str::trim).filter(|c| !c.is_empty()) {
        None => Ok(ServerTemplate::builtin(BuiltinTemplate::Fresh)),
        Some(code) if ServerTemplate::looks_like_code(code) => ServerTemplate::decode(code).map_err(|e| e.to_string()),
        Some(id) => BuiltinTemplate::from_id(id)
            .map(ServerTemplate::builtin)
            .ok_or_else(|| "unknown server template".to_string()),
    }
}

pub fn list() -> Vec<TemplateView> {
    BuiltinTemplate::ALL
        .into_iter()
        .map(|b| TemplateView::of(&ServerTemplate::builtin(b), Some(b)))
        .collect()
}

pub fn preview(code: &str) -> Act<TemplateView> {
    if !ServerTemplate::looks_like_code(code) {
        return Err("template codes start with commy-template:".into());
    }
    let template = ServerTemplate::decode(code).map_err(|e| e.to_string())?;
    Ok(TemplateView::of(&template, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_choice_means_a_fresh_server() {
        assert_eq!(resolve(None).unwrap(), ServerTemplate::builtin(BuiltinTemplate::Fresh));
        assert_eq!(resolve(Some("  ")).unwrap(), ServerTemplate::builtin(BuiltinTemplate::Fresh));
    }

    #[test]
    fn builtins_resolve_by_id_and_codes_by_prefix() {
        assert_eq!(resolve(Some("gaming")).unwrap(), ServerTemplate::builtin(BuiltinTemplate::Gaming));
        let code = ServerTemplate::builtin(BuiltinTemplate::Study).encode();
        assert_eq!(resolve(Some(&code)).unwrap(), ServerTemplate::builtin(BuiltinTemplate::Study));
        assert!(resolve(Some("arcade")).is_err());
        assert!(resolve(Some("commy-template:nope")).is_err());
    }

    #[test]
    fn the_picker_lists_every_builtin_with_a_preview() {
        let list = list();
        assert_eq!(list.len(), BuiltinTemplate::ALL.len());
        assert_eq!(list[0].id.as_deref(), Some("fresh"));
        assert!(list.iter().all(|t| !t.channels.is_empty() && !t.blurb.is_empty()));
    }

    #[test]
    fn previews_refuse_plain_text() {
        assert!(preview("hello").is_err());
        let code = ServerTemplate::builtin(BuiltinTemplate::Community).encode();
        let view = preview(&code).unwrap();
        assert_eq!(view.id, None);
        assert!(view.has_rules);
        assert_eq!(view.title, "Community");
    }
}
