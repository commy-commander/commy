use crate::state::App;
use crate::{current_profile, err_to_string, CmdResult};
use commy_core::ServerId;
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
    crate::managed(&app, sid, me.id, commy_view::templates::export)
}
