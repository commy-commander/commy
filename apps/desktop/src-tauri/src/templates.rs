use crate::state::App;
use crate::{current_profile, err_to_string, manage_check, CmdResult};
use commy_core::{Permissions, ServerId, ServerTemplate};
use commy_view::templates::TemplateView;

pub use commy_view::templates::resolve;

#[tauri::command]
pub fn list_server_templates() -> Vec<TemplateView> {
    commy_view::templates::list()
}

#[tauri::command]
pub fn preview_server_template(code: String) -> CmdResult<TemplateView> {
    commy_view::templates::preview(&code)
}

#[tauri::command]
pub fn export_server_template(app: tauri::State<'_, App>, server_id: String) -> CmdResult<String> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "copy this server as a template")?;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    ServerTemplate::from_server(&state.server).map(|t| t.encode()).map_err(err_to_string)
}
