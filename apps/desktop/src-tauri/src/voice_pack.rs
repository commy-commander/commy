use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::App;

const PACK_JSON: &str = include_str!("../voice-pack.json");
const VOICES_DIR: &str = "voices";
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Deserialize, Clone, Debug)]
pub struct PackFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct PackVoice {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub model: PackFile,
    pub config: PackFile,
}

#[derive(Deserialize, Debug)]
pub struct Pack {
    pub release: String,
    pub runtime: Vec<PackFile>,
    pub voices: Vec<PackVoice>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct VoiceStatus {
    id: String,
    name: String,
    lang: String,
    ready: bool,
    download_bytes: u64,
}

#[derive(Serialize, Debug)]
pub struct PackStatus {
    available: bool,
    voices: Vec<VoiceStatus>,
}

#[derive(Serialize, Clone, Debug)]
struct Progress<'a> {
    voice: &'a str,
    loaded: u64,
    total: u64,
}

pub fn pack() -> &'static Pack {
    static PACK: OnceLock<Pack> = OnceLock::new();
    PACK.get_or_init(|| serde_json::from_str(PACK_JSON).expect("voice-pack.json is valid"))
}

impl Pack {
    fn voice(&self, id: &str) -> Option<&PackVoice> {
        self.voices.iter().find(|v| v.id == id)
    }

    fn file(&self, name: &str) -> Option<&PackFile> {
        self.runtime
            .iter()
            .chain(self.voices.iter().flat_map(|v| [&v.model, &v.config]))
            .find(|f| f.name == name)
    }

    fn files_for<'a>(&'a self, voice: &'a PackVoice) -> impl Iterator<Item = &'a PackFile> {
        self.runtime.iter().chain([&voice.model, &voice.config])
    }

    fn url(&self, file: &PackFile) -> String {
        format!("https://github.com/{}/releases/download/{}/{}", crate::updater::REPO, self.release, file.name)
    }
}

fn voices_dir(app: &App) -> Option<PathBuf> {
    app.base_dir().map(|base| base.join(VOICES_DIR))
}

fn is_ready(dir: &Path, file: &PackFile) -> bool {
    std::fs::metadata(dir.join(&file.name)).is_ok_and(|m| m.is_file() && m.len() == file.size)
}

fn missing<'a>(dir: &Path, files: impl Iterator<Item = &'a PackFile>) -> Vec<&'a PackFile> {
    let mut seen = HashSet::new();
    files.filter(|f| seen.insert(f.name.as_str()) && !is_ready(dir, f)).collect()
}

fn status_in(dir: Option<&Path>) -> PackStatus {
    let pack = pack();
    let voices = pack
        .voices
        .iter()
        .map(|voice| {
            let needed = dir.map(|d| missing(d, pack.files_for(voice))).unwrap_or_default();
            VoiceStatus {
                id: voice.id.clone(),
                name: voice.name.clone(),
                lang: voice.lang.clone(),
                ready: dir.is_some() && needed.is_empty(),
                download_bytes: if dir.is_some() { needed.iter().map(|f| f.size).sum() } else { pack.files_for(voice).map(|f| f.size).sum() },
            }
        })
        .collect();
    PackStatus { available: dir.is_some(), voices }
}

fn hex(digest: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(digest)
}

#[tauri::command]
pub fn voice_pack_status(app: tauri::State<'_, App>) -> PackStatus {
    status_in(voices_dir(&app).as_deref())
}

#[tauri::command]
pub async fn voice_pack_download(app: tauri::State<'_, App>, handle: AppHandle, voice: String) -> Result<(), String> {
    static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let pack = pack();
    let chosen = pack.voice(&voice).ok_or("unknown voice")?;
    let dir = voices_dir(&app).ok_or("voices cannot be saved yet")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make the voices folder: {e}"))?;
    let needed: Vec<PackFile> = missing(&dir, pack.files_for(chosen)).into_iter().cloned().collect();
    if needed.is_empty() {
        return Ok(());
    }
    if !BUSY.lock().unwrap().get_or_insert_with(HashSet::new).insert(voice.clone()) {
        return Err("this voice is already downloading".into());
    }
    let result = download_all(&handle, pack, &dir, &voice, &needed).await;
    BUSY.lock().unwrap().get_or_insert_with(HashSet::new).remove(&voice);
    result
}

async fn download_all(handle: &AppHandle, pack: &Pack, dir: &Path, voice: &str, files: &[PackFile]) -> Result<(), String> {
    let total: u64 = files.iter().map(|f| f.size).sum();
    let client = reqwest::Client::builder()
        .user_agent(format!("Commy/{}", crate::updater::current_version()))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut done = 0;
    let mut last = Instant::now();
    for file in files {
        let mut response = client
            .get(pack.url(file))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| "could not download the voice, check your connection".to_string())?;
        let part = dir.join(format!("{}.part", file.name));
        let mut bytes = Vec::with_capacity(file.size as usize);
        let mut hasher = Sha256::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "the voice download was interrupted".to_string())? {
            if bytes.len() as u64 + chunk.len() as u64 > file.size {
                return Err("the voice download was not the expected file".into());
            }
            hasher.update(&chunk);
            bytes.extend_from_slice(&chunk);
            if last.elapsed() >= PROGRESS_EVERY {
                last = Instant::now();
                let _ = handle.emit("voice-pack-progress", Progress { voice, loaded: done + bytes.len() as u64, total });
            }
        }
        if bytes.len() as u64 != file.size || hex(&hasher.finalize()) != file.sha256 {
            return Err("the voice download was not the expected file".into());
        }
        std::fs::write(&part, &bytes).map_err(|e| format!("could not save the voice: {e}"))?;
        std::fs::rename(&part, dir.join(&file.name)).map_err(|e| format!("could not save the voice: {e}"))?;
        done += file.size;
    }
    let _ = handle.emit("voice-pack-progress", Progress { voice, loaded: total, total });
    Ok(())
}

#[tauri::command]
pub async fn voice_pack_file(handle: AppHandle, name: String) -> Result<tauri::ipc::Response, String> {
    let file = pack().file(&name).ok_or("unknown voice file")?;
    let dir = voices_dir(&handle.state::<App>()).ok_or("voices cannot be read yet")?;
    if !is_ready(&dir, file) {
        return Err("this voice is not downloaded".into());
    }
    let bytes = tokio::fs::read(dir.join(&file.name)).await.map_err(|e| format!("could not read the voice: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub fn voice_pack_remove(app: tauri::State<'_, App>, voice: String) -> Result<(), String> {
    let chosen = pack().voice(&voice).ok_or("unknown voice")?;
    let dir = voices_dir(&app).ok_or("voices cannot be removed yet")?;
    for file in [&chosen.model, &chosen.config] {
        let _ = std::fs::remove_file(dir.join(&file.name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pack_lists_runtime_and_voices_with_hashes() {
        let pack = pack();
        assert_eq!(pack.release, "tts-pack-1");
        assert_eq!(pack.runtime.len(), 3);
        assert!(pack.voices.len() >= 6);
        let all = pack.runtime.iter().chain(pack.voices.iter().flat_map(|v| [&v.model, &v.config]));
        let mut names = HashSet::new();
        for file in all {
            assert_eq!(file.sha256.len(), 64, "{}", file.name);
            assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(file.size > 0);
            assert!(!file.name.contains('/') && !file.name.contains('\\') && !file.name.starts_with('.'));
            assert!(names.insert(file.name.clone()), "duplicate {}", file.name);
        }
        assert!(pack.voice("en_US-kristin-medium").is_some());
        assert!(pack.url(&pack.runtime[0]).starts_with("https://github.com/"));
    }

    #[test]
    fn only_pack_files_can_be_read() {
        assert!(pack().file("piper_phonemize.data").is_some());
        assert!(pack().file("en_US-kristin-medium.onnx.json").is_some());
        assert!(pack().file("../secrets").is_none());
        assert!(pack().file("local.key").is_none());
    }

    #[test]
    fn status_counts_what_is_left_to_download() {
        let dir = std::env::temp_dir().join(format!("commy-voices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pack = pack();
        let kristin = pack.voice("en_US-kristin-medium").unwrap();
        let whole: u64 = pack.files_for(kristin).map(|f| f.size).sum();
        let before = status_in(Some(&dir));
        let first = before.voices.iter().find(|v| v.id == kristin.id).unwrap();
        assert!(!first.ready);
        assert_eq!(first.download_bytes, whole);
        std::fs::write(dir.join(&kristin.config.name), vec![0u8; kristin.config.size as usize]).unwrap();
        let after = status_in(Some(&dir));
        let second = after.voices.iter().find(|v| v.id == kristin.id).unwrap();
        assert_eq!(second.download_bytes, whole - kristin.config.size);
        std::fs::write(dir.join(&kristin.model.name), b"short").unwrap();
        assert!(!is_ready(&dir, &kristin.model));
        let unavailable = status_in(None);
        assert!(!unavailable.available && unavailable.voices.iter().all(|v| !v.ready));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
