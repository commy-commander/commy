use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[derive(Serialize)]
struct Request<'a> {
    nonce: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleCredential {
    pub identity_token: String,
    #[serde(default)]
    pub given_name: String,
    #[serde(default)]
    pub family_name: String,
}

pub struct AppleSignIn<R: Runtime>(#[cfg(target_os = "ios")] tauri::plugin::PluginHandle<R>, std::marker::PhantomData<fn() -> R>);

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_commy_apple_sign_in);

impl<R: Runtime> AppleSignIn<R> {
    #[cfg(target_os = "ios")]
    pub fn sign_in(&self, hashed_nonce: &str) -> Result<AppleCredential, String> {
        self.0.run_mobile_plugin("signIn", Request { nonce: hashed_nonce }).map_err(|e| e.to_string())
    }

    #[cfg(not(target_os = "ios"))]
    pub fn sign_in(&self, hashed_nonce: &str) -> Result<AppleCredential, String> {
        let _ = Request { nonce: hashed_nonce };
        Err("Sign in with Apple is only in the iPhone app".into())
    }

    #[cfg(target_os = "ios")]
    pub fn run(&self, command: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        self.0.run_mobile_plugin(command, args).map_err(|e| e.to_string())
    }

    #[cfg(not(target_os = "ios"))]
    pub fn run(&self, _command: &str, _args: serde_json::Value) -> Result<serde_json::Value, String> {
        Err("This sign-in is only in the phone apps".into())
    }
}

pub trait AppleSignInExt<R: Runtime> {
    fn apple_sign_in(&self) -> &AppleSignIn<R>;
}

impl<R: Runtime, T: Manager<R>> AppleSignInExt<R> for T {
    fn apple_sign_in(&self) -> &AppleSignIn<R> {
        self.state::<AppleSignIn<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("commy-apple-sign-in")
        .setup(|app, _api| {
            #[cfg(target_os = "ios")]
            app.manage(AppleSignIn(_api.register_ios_plugin(init_plugin_commy_apple_sign_in)?, std::marker::PhantomData));
            #[cfg(not(target_os = "ios"))]
            app.manage(AppleSignIn::<R>(std::marker::PhantomData));
            Ok(())
        })
        .build()
}
