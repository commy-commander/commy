use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, reload, EnvFilter, Registry};

pub const LOG_FOLDER: &str = "logs";
pub const LOG_FILE: &str = "commy.log";
pub const PREVIOUS_LOG_FILE: &str = "commy.previous.log";
const VERBOSE_FLAG: &str = "verbose";
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
const EARLY_LINES: usize = 500;
pub const QUIET_FILTER: &str = "info,iroh=warn,iroh_net=warn";
pub const VERBOSE_FILTER: &str = "debug,iroh=debug,hyper=info,hyper_util=info,reqwest=info,rustls=info,h2=info,tungstenite=info,tokio_tungstenite=info,tao=info,wry=info,mio=info,want=info";
const HIDDEN: &str = "[hidden]";
const SECRET_NAMES: [&str; 9] =
    ["token", "secret", "password", "authorization", "cookie", "api_key", "apikey", "recovery_code", "access_code"];
const SPANS_TO_LINE_END: [&str; 2] = ["authorization", "cookie"];

#[derive(Default)]
struct Sink {
    folder: Option<PathBuf>,
    file: Option<File>,
    written: u64,
    early: Vec<String>,
    home: Vec<String>,
}

static SINK: Mutex<Sink> = Mutex::new(Sink { folder: None, file: None, written: 0, early: Vec::new(), home: Vec::new() });
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogStatus {
    pub verbose: bool,
    pub saving: bool,
}

pub fn init() {
    let from_env = EnvFilter::try_from_default_env().ok();
    let (filter, handle) = reload::Layer::new(from_env.unwrap_or_else(|| EnvFilter::new(QUIET_FILTER)));
    let _ = FILTER.set(handle);
    sink().home = home_spellings(std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok().as_deref());
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stdout))
        .with(fmt::layer().with_ansi(false).with_writer(|| LogFileWriter))
        .try_init();
}

pub fn attach(base_dir: &Path) {
    let folder = base_dir.join(LOG_FOLDER);
    if let Err(e) = std::fs::create_dir_all(&folder) {
        tracing::warn!("could not create the log folder: {e}");
        return;
    }
    let verbose = folder.join(VERBOSE_FLAG).exists();
    {
        let mut sink = sink();
        let mut opened = open_log(&folder);
        if let Some((file, written)) = opened.as_mut() {
            for line in std::mem::take(&mut sink.early) {
                if file.write_all(line.as_bytes()).is_ok() {
                    *written += line.len() as u64;
                }
            }
        }
        sink.written = opened.as_ref().map_or(0, |(_, written)| *written);
        sink.file = opened.map(|(file, _)| file);
        sink.folder = Some(folder);
    }
    apply_filter(verbose);
    tracing::info!("Commy {} on {} {}, detailed logging {}", crate::updater::current_version(), std::env::consts::OS, std::env::consts::ARCH, on_off(verbose));
}

pub fn status() -> LogStatus {
    let folder = folder();
    LogStatus { verbose: folder.as_ref().is_some_and(|f| f.join(VERBOSE_FLAG).exists()), saving: folder.is_some() }
}

pub fn set_verbose(enabled: bool) -> Result<LogStatus, String> {
    let folder = folder().ok_or("Logs are not being saved on this device")?;
    let flag = folder.join(VERBOSE_FLAG);
    if enabled {
        std::fs::write(&flag, b"").map_err(|e| e.to_string())?;
    } else if flag.exists() {
        std::fs::remove_file(&flag).map_err(|e| e.to_string())?;
    }
    apply_filter(enabled);
    tracing::info!("detailed logging {}", on_off(enabled));
    Ok(status())
}

pub fn folder() -> Option<PathBuf> {
    sink().folder.clone()
}

pub fn bundle(header: &str) -> Result<String, String> {
    let folder = folder().ok_or("Logs are not being saved on this device")?;
    let home = sink().home.clone();
    let mut text = String::from(header);
    for name in [PREVIOUS_LOG_FILE, LOG_FILE] {
        let Ok(bytes) = std::fs::read(folder.join(name)) else { continue };
        text.push_str(&format!("\n===== {name} =====\n"));
        for line in String::from_utf8_lossy(&bytes).lines() {
            text.push_str(&scrub(line, &home));
            text.push('\n');
        }
    }
    Ok(text)
}

pub fn bundle_name(now: chrono::DateTime<chrono::Local>) -> String {
    format!("commy-logs-{}.txt", now.format("%Y%m%d-%H%M"))
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn sink() -> std::sync::MutexGuard<'static, Sink> {
    SINK.lock().unwrap_or_else(|e| e.into_inner())
}

fn apply_filter(verbose: bool) {
    if let Some(handle) = FILTER.get() {
        let _ = handle.reload(EnvFilter::new(if verbose { VERBOSE_FILTER } else { QUIET_FILTER }));
    }
}

fn open_log(folder: &Path) -> Option<(File, u64)> {
    let path = folder.join(LOG_FILE);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size >= MAX_LOG_BYTES {
        let _ = std::fs::rename(&path, folder.join(PREVIOUS_LOG_FILE));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
    let written = file.metadata().map(|m| m.len()).unwrap_or(0);
    Some((file, written))
}

fn write_line(text: &str) {
    let mut sink = sink();
    let line = scrub(text.trim_end_matches('\n'), &sink.home) + "\n";
    if sink.folder.is_none() {
        if sink.early.len() < EARLY_LINES {
            sink.early.push(line);
        }
        return;
    }
    if sink.written >= MAX_LOG_BYTES {
        sink.file = None;
        let reopened = sink.folder.as_deref().and_then(open_log);
        sink.written = reopened.as_ref().map_or(0, |(_, written)| *written);
        sink.file = reopened.map(|(file, _)| file);
    }
    let Some(file) = sink.file.as_mut() else { return };
    if file.write_all(line.as_bytes()).is_ok() {
        sink.written += line.len() as u64;
    }
}

struct LogFileWriter;

impl Write for LogFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        write_line(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn home_spellings(home: Option<&str>) -> Vec<String> {
    let Some(home) = home.map(|h| h.trim_end_matches(['/', '\\'])).filter(|h| h.len() > 1) else {
        return Vec::new();
    };
    let mut spellings = vec![home.replace('\\', "\\\\"), home.to_string(), home.replace('\\', "/")];
    spellings.dedup();
    spellings
}

pub fn scrub(line: &str, home: &[String]) -> String {
    let mut line = hide_secret_values(line);
    for spelling in home {
        line = line.replace(spelling.as_str(), "~");
    }
    line
}

fn hide_secret_values(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    while at < line.len() {
        if let Some((value_start, to_line_end)) = secret_value_start(&lower, at) {
            let end = value_end(line, value_start, to_line_end);
            out.push_str(&line[at..value_start]);
            if end > value_start {
                out.push_str(HIDDEN);
            }
            at = end;
            continue;
        }
        let ch = line[at..].chars().next().unwrap_or_default();
        out.push(ch);
        at += ch.len_utf8().max(1);
    }
    out
}

fn secret_value_start(lower: &str, at: usize) -> Option<(usize, bool)> {
    let rest = &lower[at..];
    if rest.starts_with("bearer ") {
        return Some((at + "bearer ".len(), false));
    }
    let name = SECRET_NAMES.iter().find(|name| rest.starts_with(**name))?;
    let bytes = lower.as_bytes();
    let mut i = at + name.len();
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
        i += 1;
    }
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    if i >= bytes.len() || !(bytes[i] == b'=' || bytes[i] == b':') {
        return None;
    }
    i += 1;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'"' || bytes[i] == b'\'') {
        i += 1;
    }
    Some((i, SPANS_TO_LINE_END.contains(name)))
}

fn value_end(line: &str, start: usize, to_line_end: bool) -> usize {
    line[start..]
        .char_indices()
        .find(|(_, ch)| {
            matches!(ch, '"' | '\'' | '\n' | '\r') || (!to_line_end && (ch.is_whitespace() || matches!(ch, '&' | ',' | ';' | ')' | '}' | ']')))
        })
        .map_or(line.len(), |(offset, _)| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(line: &str) -> String {
        scrub(line, &[])
    }

    #[test]
    fn hides_tokens_in_queries_and_json() {
        assert_eq!(clean("GET /api?token=abc123&page=2"), "GET /api?token=[hidden]&page=2");
        assert_eq!(clean(r#"{"push_token":"abc","n":1}"#), r#"{"push_token":"[hidden]","n":1}"#);
        assert_eq!(clean("session_token = xyz done"), "session_token = [hidden] done");
        assert_eq!(clean("igdb client_secret: s3cr3t"), "igdb client_secret: [hidden]");
        assert_eq!(clean("PASSWORD=hunter2"), "PASSWORD=[hidden]");
    }

    #[test]
    fn hides_headers_to_the_end_of_the_line() {
        assert_eq!(clean("Authorization: Bearer abc.def"), "Authorization: [hidden]");
        assert_eq!(clean("cookie: a=1; b=2"), "cookie: [hidden]");
        assert_eq!(clean("sent with bearer abc.def ok"), "sent with bearer [hidden] ok");
    }

    #[test]
    fn leaves_ordinary_lines_alone() {
        for line in [
            "joined voice in 4f2a, 3 peers",
            "the key for local files cannot be read",
            "tokens left 3",
            "sharing the key of abcd with your other devices",
            "relay round trip 42 ms",
        ] {
            assert_eq!(clean(line), line);
        }
    }

    #[test]
    fn keeps_multibyte_text_intact() {
        assert_eq!(clean("caf\u{e9} token=\u{e9}t\u{e9} fin"), "caf\u{e9} token=[hidden] fin");
    }

    #[test]
    fn replaces_the_home_folder_in_every_spelling() {
        let home = home_spellings(Some(r"C:\Users\Someone\"));
        assert_eq!(scrub(r"could not move C:\Users\Someone\AppData\x", &home), r"could not move ~\AppData\x");
        assert_eq!(scrub(r#"path "C:\\Users\\Someone\\AppData""#, &home), r#"path "~\\AppData""#);
        assert_eq!(scrub("C:/Users/Someone/AppData", &home), "~/AppData");
        let unix = home_spellings(Some("/home/someone"));
        assert_eq!(scrub("/home/someone/.local/share/commy", &unix), "~/.local/share/commy");
        assert!(home_spellings(Some("/")).is_empty());
        assert!(home_spellings(None).is_empty());
    }

    #[test]
    fn bundle_names_sort_by_time() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-26T01:02:03+00:00").unwrap().with_timezone(&chrono::Local);
        let name = bundle_name(at);
        assert!(name.starts_with("commy-logs-2026") && name.ends_with(".txt"));
    }

    #[test]
    fn saved_logs_keep_early_lines_and_hide_secrets() {
        let dir = std::env::temp_dir().join(format!("commy-log-bundle-{}", rand::random::<u64>()));
        write_line("starting up with push_token=early-secret\n");
        attach(&dir);
        write_line("signed in, cookie: session=abc\n");
        let saved = bundle("header\n").unwrap();
        assert!(saved.starts_with("header\n"));
        assert!(saved.contains("push_token=[hidden]"));
        assert!(saved.contains("cookie: [hidden]"));
        assert!(!saved.contains("early-secret") && !saved.contains("session=abc"));
        assert!(!status().verbose);
        assert!(set_verbose(true).unwrap().verbose);
        assert!(dir.join(LOG_FOLDER).join(VERBOSE_FLAG).exists());
        assert!(!set_verbose(false).unwrap().verbose);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotation_keeps_one_previous_file() {
        let dir = std::env::temp_dir().join(format!("commy-log-test-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOG_FILE), vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        let (_, written) = open_log(&dir).unwrap();
        assert_eq!(written, 0);
        assert_eq!(std::fs::metadata(dir.join(PREVIOUS_LOG_FILE)).unwrap().len(), MAX_LOG_BYTES);
        std::fs::remove_dir_all(&dir).ok();
    }
}
