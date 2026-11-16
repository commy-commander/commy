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

#[cfg(target_os = "ios")]
static REOPEN_NEEDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "ios")]
pub fn restart(handle: &AppHandle) -> ! {
    use tauri::Emitter;
    crate::invite_link::step_aside();
    REOPEN_NEEDED.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = handle.emit("reopen-needed", ());
    loop {
        std::thread::park();
    }
}

#[cfg(target_os = "ios")]
pub fn reopen_needed() -> bool {
    REOPEN_NEEDED.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(not(target_os = "ios"))]
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
