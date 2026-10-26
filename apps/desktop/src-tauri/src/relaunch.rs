use tauri::AppHandle;

#[cfg(target_os = "android")]
static PLUGIN: std::sync::OnceLock<tauri::plugin::PluginHandle<tauri::Wry>> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("relaunch")
        .setup(|_app, api| {
            let _ = PLUGIN.set(api.register_android_plugin("chat.commy.mobile", "RelaunchPlugin")?);
            Ok(())
        })
        .build()
}

pub fn restart(handle: &AppHandle) -> ! {
    crate::invite_link::step_aside();
    #[cfg(target_os = "android")]
    if let Some(plugin) = PLUGIN.get() {
        match plugin.run_mobile_plugin::<serde_json::Value>("relaunch", serde_json::json!({})) {
            Ok(_) => std::thread::sleep(std::time::Duration::from_secs(5)),
            Err(e) => tracing::warn!("could not reopen Commy: {e}"),
        }
    }
    handle.restart()
}
