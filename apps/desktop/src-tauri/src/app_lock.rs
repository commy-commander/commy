use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::lock_or_recover::LockOrRecover;

const SETTINGS_FILE: &str = "app-lock.json";
pub const LOCK_AFTER_CHOICES: [u64; 5] = [0, 60, 300, 900, 3600];
const CHANGED_EVENT: &str = "app-lock-changed";
const UNLOCK_REASON: &str = "Unlock Commy";
const TURN_ON_REASON: &str = "Turn on the app lock";
const TURN_OFF_REASON: &str = "Turn off the app lock";
const CANCELED: &str = "canceled";
const NO_PHONE_LOCK: &str = "no-lock";
const SET_A_PHONE_LOCK: &str = "Set a passcode on your phone first, then turn this on.";
const PHONE_LOCK_REMOVED: &str = "Your phone has no passcode now, so the app lock turned off.";
const NOT_A_CHOICE: &str = "That lock time is not one of the choices";
const ALREADY_ASKING: &str = "The unlock prompt is already open";
const HELPER_STOPPED: &str = "The app lock helper stopped unexpectedly";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockSettings {
    pub enabled: bool,
    pub after_secs: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PhoneCheck {
    pub secure: bool,
    pub biometry: String,
}

impl PhoneCheck {
    fn unlock_with(&self) -> &str {
        match (self.secure, self.biometry.as_str()) {
            (false, _) => "none",
            (true, "" | "none") => "passcode",
            (true, kind) => kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockStatus {
    pub supported: bool,
    pub phone_secure: bool,
    pub unlock_with: String,
    pub enabled: bool,
    pub after_secs: u64,
    pub locked: bool,
    pub asking: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

#[derive(Debug)]
pub struct Lock {
    settings: LockSettings,
    locked: bool,
    away_since: Option<Instant>,
    asking: bool,
}

impl Lock {
    pub const fn off() -> Self {
        Self { settings: LockSettings { enabled: false, after_secs: 0 }, locked: false, away_since: None, asking: false }
    }

    pub fn opened_with(settings: LockSettings) -> Self {
        Self { settings, locked: settings.enabled, ..Self::off() }
    }

    pub fn went_away(&mut self, now: Instant) {
        if self.settings.enabled && !self.asking && self.away_since.is_none() {
            self.away_since = Some(now);
        }
    }

    pub fn came_back(&mut self, now: Instant) -> bool {
        let Some(since) = self.away_since.take() else { return false };
        if !self.settings.enabled || self.locked || self.asking {
            return false;
        }
        self.locked = now.saturating_duration_since(since) >= Duration::from_secs(self.settings.after_secs);
        self.locked
    }

    pub fn begin_asking(&mut self) -> bool {
        if self.asking {
            return false;
        }
        self.asking = true;
        true
    }

    pub fn begin_unlock(&mut self) -> bool {
        self.locked && self.begin_asking()
    }

    pub fn stop_asking(&mut self) {
        self.asking = false;
    }

    pub fn unlocked(&mut self) {
        self.asking = false;
        self.locked = false;
        self.away_since = None;
    }

    pub fn apply(&mut self, settings: LockSettings) {
        self.settings = settings;
        if !settings.enabled {
            self.locked = false;
            self.away_since = None;
        }
    }

    pub fn settings(&self) -> LockSettings {
        self.settings
    }

    fn status(&self, check: &PhoneCheck, notice: Option<&str>) -> LockStatus {
        LockStatus {
            supported: cfg!(mobile),
            phone_secure: check.secure,
            unlock_with: check.unlock_with().to_string(),
            enabled: self.settings.enabled,
            after_secs: self.settings.after_secs,
            locked: self.locked,
            asking: self.asking,
            notice: notice.map(str::to_string),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Passed,
    Canceled,
    NoPhoneLock,
    Failed(String),
}

fn answer_from(result: Result<Value, String>) -> Answer {
    match result {
        Ok(_) => Answer::Passed,
        Err(e) if e.contains(NO_PHONE_LOCK) => Answer::NoPhoneLock,
        Err(e) if e.contains(CANCELED) => Answer::Canceled,
        Err(e) => Answer::Failed(e),
    }
}

pub fn valid_lock_after(secs: u64) -> bool {
    LOCK_AFTER_CHOICES.contains(&secs)
}

static LOCK: Mutex<Lock> = Mutex::new(Lock::off());
static SETTINGS_DIR: OnceLock<PathBuf> = OnceLock::new();

fn settings_file(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

fn load(dir: &Path) -> LockSettings {
    crate::persist::read_json::<LockSettings>(&settings_file(dir))
        .filter(|saved| valid_lock_after(saved.after_secs))
        .unwrap_or_default()
}

fn save(settings: LockSettings) -> Result<(), String> {
    let dir = SETTINGS_DIR.get().ok_or("Commy has no folder to keep this setting in")?;
    crate::persist::write_json(&settings_file(dir), &settings).map_err(|e| format!("Commy could not save the app lock setting: {e}"))
}

pub fn start(handle: &AppHandle, dir: &Path) {
    if !cfg!(mobile) {
        return;
    }
    let _ = SETTINGS_DIR.set(dir.to_path_buf());
    let settings = load(dir);
    *LOCK.lock_or_recover() = Lock::opened_with(settings);
    set_cover(handle, settings.enabled);
}

pub fn went_away() {
    LOCK.lock_or_recover().went_away(Instant::now());
}

#[cfg_attr(not(mobile), allow(dead_code))]
pub fn came_back(handle: &AppHandle) {
    if LOCK.lock_or_recover().came_back(Instant::now()) {
        let _ = handle.emit(CHANGED_EVENT, ());
    }
}

#[cfg(mobile)]
fn run_native(handle: &AppHandle, command: &str, args: Value) -> Result<Value, String> {
    use commy_app_lock::PhoneLockExt;
    handle.phone_lock().run(command, args)
}

#[cfg(not(mobile))]
fn run_native(_handle: &AppHandle, _command: &str, _args: Value) -> Result<Value, String> {
    Err("The app lock is only in the phone apps".into())
}

async fn on_phone(handle: &AppHandle, command: &'static str, args: Value) -> Result<Value, String> {
    let handle = handle.clone();
    tauri::async_runtime::spawn_blocking(move || run_native(&handle, command, args)).await.map_err(|_| HELPER_STOPPED.to_string())?
}

async fn check_phone(handle: &AppHandle) -> PhoneCheck {
    if !cfg!(mobile) {
        return PhoneCheck::default();
    }
    on_phone(handle, "check", json!({})).await.ok().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default()
}

async fn ask_phone(handle: &AppHandle, reason: &str) -> Answer {
    answer_from(on_phone(handle, "unlock", json!({ "reason": reason })).await)
}

fn set_cover(handle: &AppHandle, on: bool) {
    if !cfg!(mobile) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = on_phone(&handle, "cover", json!({ "on": on })).await {
            tracing::warn!("app switcher cover not set: {e}");
        }
    });
}

async fn current_status(handle: &AppHandle, notice: Option<&str>) -> LockStatus {
    let check = check_phone(handle).await;
    LOCK.lock_or_recover().status(&check, notice)
}

#[tauri::command]
pub async fn app_lock_status(handle: AppHandle) -> LockStatus {
    current_status(&handle, None).await
}

#[tauri::command]
pub fn app_lock_away() {
    went_away();
}

#[tauri::command]
pub async fn app_lock_back(handle: AppHandle) -> LockStatus {
    LOCK.lock_or_recover().came_back(Instant::now());
    current_status(&handle, None).await
}

#[tauri::command]
pub async fn app_lock_unlock(handle: AppHandle) -> Result<LockStatus, String> {
    if !LOCK.lock_or_recover().begin_unlock() {
        return Ok(current_status(&handle, None).await);
    }
    match ask_phone(&handle, UNLOCK_REASON).await {
        Answer::Passed => LOCK.lock_or_recover().unlocked(),
        Answer::NoPhoneLock => {
            LOCK.lock_or_recover().unlocked();
            turn_off(&handle)?;
            return Ok(current_status(&handle, Some(PHONE_LOCK_REMOVED)).await);
        }
        Answer::Canceled => LOCK.lock_or_recover().stop_asking(),
        Answer::Failed(e) => {
            LOCK.lock_or_recover().stop_asking();
            return Err(e);
        }
    }
    Ok(current_status(&handle, None).await)
}

fn turn_off(handle: &AppHandle) -> Result<(), String> {
    let settings = LockSettings { enabled: false, ..LOCK.lock_or_recover().settings() };
    save(settings)?;
    LOCK.lock_or_recover().apply(settings);
    set_cover(handle, false);
    Ok(())
}

async fn confirm_switch(handle: &AppHandle, turning_on: bool) -> Result<bool, String> {
    if !LOCK.lock_or_recover().begin_asking() {
        return Err(ALREADY_ASKING.into());
    }
    let answer = ask_phone(handle, if turning_on { TURN_ON_REASON } else { TURN_OFF_REASON }).await;
    LOCK.lock_or_recover().stop_asking();
    match answer {
        Answer::Passed => Ok(true),
        Answer::NoPhoneLock if turning_on => Err(SET_A_PHONE_LOCK.into()),
        Answer::NoPhoneLock => Ok(true),
        Answer::Canceled => Ok(false),
        Answer::Failed(e) => Err(e),
    }
}

#[tauri::command]
pub async fn app_lock_set(handle: AppHandle, enabled: bool, after_secs: u64) -> Result<LockStatus, String> {
    if !cfg!(mobile) {
        return Err("The app lock is only in the phone apps".into());
    }
    if !valid_lock_after(after_secs) {
        return Err(NOT_A_CHOICE.into());
    }
    let wanted = LockSettings { enabled, after_secs };
    let switching = LOCK.lock_or_recover().settings().enabled != enabled;
    if switching && !confirm_switch(&handle, enabled).await? {
        return Ok(current_status(&handle, None).await);
    }
    save(wanted)?;
    LOCK.lock_or_recover().apply(wanted);
    set_cover(&handle, enabled);
    Ok(current_status(&handle, None).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(after_secs: u64) -> LockSettings {
        LockSettings { enabled: true, after_secs }
    }

    fn unlocked_with(settings: LockSettings) -> Lock {
        let mut lock = Lock::opened_with(settings);
        lock.unlocked();
        lock
    }

    #[test]
    fn opening_the_app_starts_locked_only_when_the_lock_is_on() {
        assert!(Lock::opened_with(on(0)).locked);
        assert!(!Lock::opened_with(LockSettings::default()).locked);
    }

    #[test]
    fn locks_right_away_when_set_to_immediately() {
        let mut lock = unlocked_with(on(0));
        let left = Instant::now();
        lock.went_away(left);
        assert!(lock.came_back(left));
        assert!(lock.locked);
    }

    #[test]
    fn a_short_trip_away_stays_unlocked_and_a_long_one_locks() {
        let left = Instant::now();
        let mut lock = unlocked_with(on(60));
        lock.went_away(left);
        assert!(!lock.came_back(left + Duration::from_secs(59)));
        assert!(!lock.locked);
        lock.went_away(left);
        assert!(lock.came_back(left + Duration::from_secs(60)));
        assert!(lock.locked);
    }

    #[test]
    fn the_first_time_away_counts_when_leaving_is_heard_twice() {
        let left = Instant::now();
        let mut lock = unlocked_with(on(60));
        lock.went_away(left);
        lock.went_away(left + Duration::from_secs(50));
        assert!(lock.came_back(left + Duration::from_secs(61)));
    }

    #[test]
    fn coming_back_without_leaving_changes_nothing() {
        let mut lock = unlocked_with(on(0));
        assert!(!lock.came_back(Instant::now()));
        assert!(!lock.locked);
    }

    #[test]
    fn the_unlock_prompt_taking_the_screen_does_not_lock_again() {
        let left = Instant::now();
        let mut lock = Lock::opened_with(on(0));
        assert!(lock.begin_unlock());
        lock.went_away(left);
        assert!(!lock.came_back(left + Duration::from_secs(5)));
        lock.unlocked();
        assert!(!lock.locked);
        assert!(!lock.came_back(left + Duration::from_secs(10)));
    }

    #[test]
    fn only_one_prompt_at_a_time_and_only_while_locked() {
        let mut lock = Lock::opened_with(on(0));
        assert!(lock.begin_unlock());
        assert!(!lock.begin_unlock());
        lock.stop_asking();
        assert!(lock.locked);
        assert!(lock.begin_unlock());
        lock.unlocked();
        assert!(!lock.begin_unlock());
    }

    #[test]
    fn turning_the_lock_off_unlocks_and_forgets_the_time_away() {
        let left = Instant::now();
        let mut lock = Lock::opened_with(on(0));
        lock.went_away(left);
        lock.apply(LockSettings { enabled: false, after_secs: 0 });
        assert!(!lock.locked);
        assert!(!lock.came_back(left + Duration::from_secs(3600)));
    }

    #[test]
    fn turning_the_lock_on_does_not_lock_the_open_app() {
        let mut lock = Lock::off();
        lock.apply(on(0));
        assert!(!lock.locked);
        assert_eq!(lock.settings(), on(0));
    }

    #[test]
    fn nothing_locks_while_the_lock_is_off() {
        let left = Instant::now();
        let mut lock = Lock::off();
        lock.went_away(left);
        assert!(!lock.came_back(left + Duration::from_secs(3600)));
    }

    #[test]
    fn lock_times_are_limited_to_the_choices() {
        for secs in LOCK_AFTER_CHOICES {
            assert!(valid_lock_after(secs));
        }
        assert!(!valid_lock_after(30));
        assert!(!valid_lock_after(u64::MAX));
    }

    #[test]
    fn saved_settings_fill_in_what_is_missing() {
        let parsed: LockSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(parsed, on(0));
        let parsed: LockSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, LockSettings::default());
    }

    #[test]
    fn settings_that_were_saved_come_back_the_same() {
        let dir = std::env::temp_dir().join(format!("commy-app-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persist::write_json(&settings_file(&dir), &on(300)).unwrap();
        assert_eq!(load(&dir), on(300));
        crate::persist::write_json(&settings_file(&dir), &on(42)).unwrap();
        assert_eq!(load(&dir), LockSettings::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_button_names_the_way_the_phone_unlocks() {
        let check = |secure, biometry: &str| PhoneCheck { secure, biometry: biometry.into() };
        assert_eq!(check(true, "face").unlock_with(), "face");
        assert_eq!(check(true, "fingerprint").unlock_with(), "fingerprint");
        assert_eq!(check(true, "none").unlock_with(), "passcode");
        assert_eq!(check(true, "").unlock_with(), "passcode");
        assert_eq!(check(false, "face").unlock_with(), "none");
    }

    #[test]
    fn phone_answers_are_sorted_into_what_happens_next() {
        assert_eq!(answer_from(Ok(Value::Null)), Answer::Passed);
        assert_eq!(answer_from(Err("canceled".into())), Answer::Canceled);
        assert_eq!(answer_from(Err("no-lock".into())), Answer::NoPhoneLock);
        assert_eq!(answer_from(Err("Too many tries".into())), Answer::Failed("Too many tries".into()));
    }

    #[test]
    fn status_reports_the_lock_and_the_phone() {
        let lock = Lock::opened_with(on(60));
        let status = lock.status(&PhoneCheck { secure: true, biometry: "face".into() }, Some("hello"));
        assert!(status.locked && status.enabled && status.phone_secure);
        assert_eq!(status.after_secs, 60);
        assert_eq!(status.unlock_with, "face");
        assert_eq!(status.notice.as_deref(), Some("hello"));
        let json = serde_json::to_value(lock.status(&PhoneCheck::default(), None)).unwrap();
        assert!(json.get("notice").is_none());
    }
}
