use serde_json::{json, Value};
use tauri::AppHandle;

#[cfg(target_os = "android")]
static PLUGIN: std::sync::OnceLock<tauri::plugin::PluginHandle<tauri::Wry>> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("commy-share")
        .setup(|_app, api| {
            let _ = PLUGIN.set(api.register_android_plugin("chat.commy.mobile", "SharePlugin")?);
            Ok(())
        })
        .build()
}

fn shareable(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("commy://")) && !url.chars().any(char::is_whitespace)
}

fn shared(answer: &Value) -> bool {
    answer.get("shared").and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(target_os = "android")]
fn open_share_menu(_handle: &AppHandle, args: Value) -> Result<Value, String> {
    let plugin = PLUGIN.get().ok_or("The share menu isn't ready yet")?;
    plugin.run_mobile_plugin::<Value>("shareLink", args).map_err(|e| e.to_string())
}

#[cfg(target_os = "ios")]
fn open_share_menu(handle: &AppHandle, args: Value) -> Result<Value, String> {
    use commy_apple_sign_in::AppleSignInExt;
    handle.apple_sign_in().run("shareLink", args)
}

#[cfg(not(mobile))]
fn open_share_menu(_handle: &AppHandle, _args: Value) -> Result<Value, String> {
    Err("The share menu is only in the phone apps".into())
}

#[tauri::command]
pub async fn share_link(handle: AppHandle, url: String, title: String) -> Result<bool, String> {
    if !shareable(&url) {
        return Err("That isn't a link Commy can share".into());
    }
    let args = json!({ "url": url, "title": title });
    let answer = tauri::async_runtime::spawn_blocking(move || open_share_menu(&handle, args))
        .await
        .map_err(|_| "The share menu stopped unexpectedly".to_string())??;
    Ok(shared(&answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_invite_links_go_to_the_share_menu() {
        assert!(shareable("https://commy.chat/i/pixel-pioneers#Zx9-_Zx9-_Zx9-_Zx9-_ab"));
        assert!(shareable("commy://i/commy.chat/team42#Zx9-_Zx9-_Zx9-_Zx9-_ab"));
        assert!(!shareable("http://commy.chat/i/pixel-pioneers"));
        assert!(!shareable("javascript:alert(1)"));
        assert!(!shareable("https://commy.chat/i/pixel pioneers"));
        assert!(!shareable(""));
    }

    #[test]
    fn a_closed_menu_counts_as_not_shared() {
        assert!(shared(&json!({ "shared": true })));
        assert!(!shared(&json!({ "shared": false })));
        assert!(!shared(&json!(null)));
    }
}
