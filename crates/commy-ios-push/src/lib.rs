#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_commy_push);

#[cfg(target_os = "ios")]
pub fn register<R: tauri::Runtime, C: serde::de::DeserializeOwned>(
    api: &tauri::plugin::PluginApi<R, C>,
) -> Result<tauri::plugin::PluginHandle<R>, tauri::plugin::mobile::PluginInvokeError> {
    api.register_ios_plugin(init_plugin_commy_push)
}
