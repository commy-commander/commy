use serde_json::Value;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

pub struct PhoneLock<R: Runtime>(
    #[cfg(any(target_os = "ios", target_os = "android"))] tauri::plugin::PluginHandle<R>,
    std::marker::PhantomData<fn() -> R>,
);

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_commy_app_lock);

impl<R: Runtime> PhoneLock<R> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub fn run(&self, command: &str, args: Value) -> Result<Value, String> {
        self.0.run_mobile_plugin(command, args).map_err(|e| e.to_string())
    }

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    pub fn run(&self, _command: &str, _args: Value) -> Result<Value, String> {
        Err("The app lock is only in the phone apps".into())
    }
}

pub trait PhoneLockExt<R: Runtime> {
    fn phone_lock(&self) -> &PhoneLock<R>;
}

impl<R: Runtime, T: Manager<R>> PhoneLockExt<R> for T {
    fn phone_lock(&self) -> &PhoneLock<R> {
        self.state::<PhoneLock<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("commy-app-lock")
        .setup(|app, _api| {
            #[cfg(target_os = "ios")]
            app.manage(PhoneLock(_api.register_ios_plugin(init_plugin_commy_app_lock)?, std::marker::PhantomData));
            #[cfg(target_os = "android")]
            app.manage(PhoneLock(_api.register_android_plugin("chat.commy.mobile", "AppLockPlugin")?, std::marker::PhantomData));
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            app.manage(PhoneLock::<R>(std::marker::PhantomData));
            Ok(())
        })
        .build()
}
