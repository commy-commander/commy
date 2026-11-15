use chrono::{DateTime, Utc};
use std::any::Any;
use std::fs::OpenOptions;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const UNNAMED_THREAD: &str = "unnamed";
const UNKNOWN_PLACE: &str = "an unknown place";
const NO_MESSAGE: &str = "a panic without a message";
const LINE_BREAK_STAND_IN: &str = " | ";

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn install() {
    let home = crate::debug_log::home_spellings(home_folder().as_deref());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record(info, &home);
        previous(info);
    }));
}

pub fn write_to(log_file: PathBuf) {
    let _ = LOG_PATH.set(log_file);
}

fn home_folder() -> Option<String> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()
}

fn record(info: &PanicHookInfo<'_>, home: &[String]) {
    let current = std::thread::current();
    let report = describe(current.name().unwrap_or(UNNAMED_THREAD), &place_of(info), message_of(info.payload()));
    let report = crate::debug_log::scrub(&report, home);
    match LOG_PATH.get() {
        Some(path) => {
            let _ = append_line(path, &log_line(&report, Utc::now()));
        }
        None => tracing::error!("{report}"),
    }
}

fn place_of(info: &PanicHookInfo<'_>) -> String {
    info.location()
        .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()))
        .unwrap_or_else(|| UNKNOWN_PLACE.to_string())
}

pub fn message_of(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or(NO_MESSAGE)
}

pub fn describe(thread: &str, place: &str, message: &str) -> String {
    let message = message.trim().replace("\r\n", "\n").replace('\n', LINE_BREAK_STAND_IN);
    format!("thread '{thread}' panicked at {place}: {message}")
}

pub fn log_line(report: &str, at: DateTime<Utc>) -> String {
    format!("{} ERROR {}: {report}\n", at.format("%Y-%m-%dT%H:%M:%S%.6fZ"), module_path!())
}

pub fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.flush()?;
    file.sync_data()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn scratch_file(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("commy-panic-log-{name}-{}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("commy.log")
    }

    #[test]
    fn a_report_names_the_thread_the_place_and_the_message() {
        let report = describe("tokio-runtime-worker", "src/dm_sync.rs:12:5", "index out of bounds");
        assert_eq!(report, "thread 'tokio-runtime-worker' panicked at src/dm_sync.rs:12:5: index out of bounds");
    }

    #[test]
    fn a_multi_line_message_stays_on_one_log_line() {
        let report = describe("main", "src/lib.rs:1:1", "first\r\nsecond\nthird\n");
        assert_eq!(report, "thread 'main' panicked at src/lib.rs:1:1: first | second | third");
    }

    #[test]
    fn messages_come_out_of_str_and_string_payloads() {
        let from_str: Box<dyn Any + Send> = Box::new("called `Option::unwrap()` on a `None` value");
        let from_string: Box<dyn Any + Send> = Box::new(String::from("formatted 42"));
        let from_other: Box<dyn Any + Send> = Box::new(42_u32);
        assert_eq!(message_of(from_str.as_ref()), "called `Option::unwrap()` on a `None` value");
        assert_eq!(message_of(from_string.as_ref()), "formatted 42");
        assert_eq!(message_of(from_other.as_ref()), NO_MESSAGE);
    }

    #[test]
    fn a_log_line_matches_the_tracing_layout() {
        let at = Utc.with_ymd_and_hms(2026, 11, 15, 19, 47, 35).unwrap();
        let line = log_line("thread 'main' panicked at a.rs:1:2: boom", at);
        assert_eq!(line, "2026-11-15T19:47:35.000000Z ERROR commy_desktop::panic_log: thread 'main' panicked at a.rs:1:2: boom\n");
    }

    #[test]
    fn lines_are_appended_after_what_the_log_already_holds() {
        let path = scratch_file("append");
        std::fs::write(&path, "earlier line\n").unwrap();
        append_line(&path, "first panic\n").unwrap();
        append_line(&path, "second panic\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "earlier line\nfirst panic\nsecond panic\n");
    }

    #[test]
    fn a_missing_log_file_is_created() {
        let path = scratch_file("create");
        append_line(&path, "only line\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "only line\n");
    }

    #[test]
    fn home_folders_are_scrubbed_from_reports() {
        let home = crate::debug_log::home_spellings(Some("C:\\Users\\someone"));
        let report = describe("main", "C:\\Users\\someone\\src\\a.rs:1:1", "boom");
        let scrubbed = crate::debug_log::scrub(&report, &home);
        assert!(!scrubbed.contains("someone"));
    }
}
