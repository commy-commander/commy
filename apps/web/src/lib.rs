pub mod account;
pub mod client;
pub mod dm_book;
pub mod files;
pub mod knocks;
pub mod mic_gate;
pub mod recording;
pub mod sharing;
pub mod store;
pub mod sync_book;
pub mod voice_prefs;
pub mod watching;

#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod session;

#[cfg(target_arch = "wasm32")]
mod bindings {
    use std::rc::Rc;

    use wasm_bindgen::prelude::*;

    use crate::host::Host;
    use crate::session::Session;

    #[wasm_bindgen]
    pub struct WebClient {
        session: Rc<Session>,
    }

    #[wasm_bindgen]
    impl WebClient {
        #[wasm_bindgen(constructor)]
        pub fn new(host: Host, origin: String, relays: Option<String>) -> WebClient {
            console_error_panic_hook::set_once();
            WebClient { session: Session::new(host, origin, relays) }
        }

        pub fn invoke(&self, cmd: String, args: String) -> js_sys::Promise {
            let session = self.session.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                match session.invoke(&cmd, &args).await {
                    Ok(value) => Ok(JsValue::from_str(&value.to_string())),
                    Err(message) => Err(JsValue::from_str(&message)),
                }
            })
        }

        pub fn attach_dm(&self, user_id: String, filename: String, data: Vec<u8>) -> js_sys::Promise {
            let session = self.session.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                match session.attach_dm(&user_id, &filename, data).await {
                    Ok(value) => Ok(JsValue::from_str(&value.to_string())),
                    Err(message) => Err(JsValue::from_str(&message)),
                }
            })
        }

        pub fn attach(&self, server_id: String, channel_id: String, filename: String, data: Vec<u8>) -> js_sys::Promise {
            let session = self.session.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                match session.attach_server(&server_id, &channel_id, &filename, data).await {
                    Ok(value) => Ok(JsValue::from_str(&value.to_string())),
                    Err(message) => Err(JsValue::from_str(&message)),
                }
            })
        }

        pub fn drive_upload(&self, server_id: String, channel_id: String, parent_id: Option<String>, filename: String, data: Vec<u8>) -> js_sys::Promise {
            let session = self.session.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                match session.drive_upload(&server_id, &channel_id, parent_id, &filename, data).await {
                    Ok(value) => Ok(JsValue::from_str(&value.to_string())),
                    Err(message) => Err(JsValue::from_str(&message)),
                }
            })
        }

        pub fn file_bytes(&self, cmd: String, args: String) -> js_sys::Promise {
            let session = self.session.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                match session.file_bytes(&cmd, &args).await {
                    Ok(bytes) => Ok(js_sys::Uint8Array::from(&bytes[..]).into()),
                    Err(message) => Err(JsValue::from_str(&message)),
                }
            })
        }

        pub fn voice_frame(&self, pcm: &[i16]) {
            self.session.voice_frame(pcm);
        }

        pub fn filter_test_frame(&self, pcm: &[i16]) -> Vec<i16> {
            self.session.filter_test_frame(pcm)
        }

        pub fn screen_frame(&self, width: u32, height: u32, jpeg: Vec<u8>) -> Result<u32, JsValue> {
            self.session.screen_frame(width, height, jpeg).map(|viewers| viewers as u32).map_err(|message| JsValue::from_str(&message))
        }
    }
}
