use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    pub type Host;

    #[wasm_bindgen(method, catch)]
    pub async fn request(this: &Host, method: &str, path: &str, body: Option<String>) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(method, catch, js_name = fetchBytes)]
    pub async fn fetch_bytes(this: &Host, url: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(method)]
    pub fn load(this: &Host, key: &str) -> Option<String>;

    #[wasm_bindgen(method)]
    pub fn save(this: &Host, key: &str, value: &str);

    #[wasm_bindgen(method)]
    pub fn remove(this: &Host, key: &str);

    #[wasm_bindgen(method)]
    pub fn emit(this: &Host, event: &str, payload: &str);

    #[wasm_bindgen(method, catch, js_name = startVoice)]
    pub async fn start_voice(this: &Host) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(method, js_name = stopVoice)]
    pub fn stop_voice(this: &Host);

    #[wasm_bindgen(method, js_name = playVoice)]
    pub fn play_voice(this: &Host, peer: &str, pcm: &[i16]);
}
