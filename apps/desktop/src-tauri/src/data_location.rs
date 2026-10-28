#![cfg_attr(mobile, allow(dead_code))]

use crate::state::App;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

pub const DATA_DIR_VAR: &str = "COMMY_DATA_DIR";
pub const RUNNING_LOCK_FILE: &str = "running.lock";
const WEBVIEW_FOLDER_VAR: &str = "WEBVIEW2_USER_DATA_FOLDER";
pub const POINTER_FILE: &str = "data-folder.txt";
const PENDING_FILE: &str = "data-folder-pending.json";
pub const NOTIFICATION_ICON_FILE: &str = "notification-icon.png";
pub const NEW_FOLDER_NAME: &str = "Commy";
const WEBVIEW_ENGINE_DIR: &str = "EBWebView";
const SPARE_BYTES: u64 = 256 * 1024 * 1024;
const COMPARE_CHUNK: usize = 256 * 1024;
const PROBE_FILE: &str = ".commy-write-check";

pub const WEBVIEW_CACHE_DIRS: &[&str] = &[
    "Cache",
    "Code Cache",
    "GPUCache",
    "GrShaderCache",
    "GraphiteDawnCache",
    "ShaderCache",
    "DawnCache",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "Crashpad",
];

const STAYS_PUT: &[&str] = &[RUNNING_LOCK_FILE, POINTER_FILE, PENDING_FILE, NOTIFICATION_ICON_FILE];

const COMMY_ENTRIES: &[&str] = &[
    crate::account_folder::ACCOUNTS_DIR,
    crate::account::FILE,
    crate::account::PENDING_SIGN_IN_FILE,
    crate::account_folder::SHOW_WINDOW_FILE,
    crate::local_key::KEY_FILE,
    crate::local_key::SET_ASIDE_DIR,
    crate::invite_link::PORT_FILE,
    crate::link_safety::LIST_FILE,
    crate::push_wake::KEPT_FILE,
    crate::debug_log::LOG_FOLDER,
    crate::voice_pack::VOICES_DIR,
];

const COMMY_ENTRY_PATTERNS: &[(&str, &str)] = &[("entry-key-", ".json"), ("taskbar-icon-", ".ico")];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Default,
    Chosen,
    Overridden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub home: PathBuf,
    pub folder: PathBuf,
    pub kind: Kind,
}

impl Location {
    fn at_home(&self) -> bool {
        same_path(&self.folder, &self.home)
    }

    fn uses_own_webview_folder(&self) -> bool {
        self.kind != Kind::Default || crate::instance::TEST_BUILD
    }

    pub fn webview_engine_dir(&self) -> Option<PathBuf> {
        if !cfg!(windows) {
            return None;
        }
        let root = if self.uses_own_webview_folder() {
            self.folder.join(crate::instance::WEBVIEW_DIR)
        } else {
            local_app_data()?.join(crate::toast::APP_ID)
        };
        Some(root.join(WEBVIEW_ENGINE_DIR))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Pending {
    Move { to: PathBuf },
    ClearWebCache,
    ClearEverything,
}

static CURRENT: Mutex<Option<Location>> = Mutex::new(None);

pub fn current() -> Option<Location> {
    CURRENT.lock().unwrap().clone()
}

fn set_current(location: Location) {
    *CURRENT.lock().unwrap() = Some(location);
}

pub fn folder_from_environment() -> Option<String> {
    let set_by_hand = current().is_none_or(|location| location.kind == Kind::Overridden);
    std::env::var(DATA_DIR_VAR).ok().filter(|_| set_by_hand)
}

pub fn platform_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join(crate::toast::APP_ID))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support").join(crate::toast::APP_ID))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
        Some(base.join(crate::toast::APP_ID))
    }
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

pub fn home_dir() -> Option<PathBuf> {
    let dir = platform_dir()?;
    Some(if crate::instance::TEST_BUILD { dir.join(crate::instance::TEST_BUILD_DIR) } else { dir })
}

pub fn decide(home: &Path, env: Option<&Path>, pointer: Option<PathBuf>) -> Location {
    let env = env.filter(|dir| !dir.as_os_str().is_empty());
    let pointer = pointer.filter(|dir| usable_folder(dir));
    if let Some(env) = env.filter(|env| !same_path(env, home) && !pointer.as_deref().is_some_and(|p| same_path(env, p))) {
        return Location { home: home.into(), folder: env.into(), kind: Kind::Overridden };
    }
    match pointer {
        Some(chosen) if !same_path(&chosen, home) => Location { home: home.into(), folder: chosen, kind: Kind::Chosen },
        _ => Location { home: home.into(), folder: home.into(), kind: Kind::Default },
    }
}

fn usable_folder(dir: &Path) -> bool {
    dir.is_absolute() && dir.parent().is_some()
}

pub fn resolve_before_start() -> Option<Location> {
    let home = home_dir()?;
    let env = std::env::var_os(DATA_DIR_VAR).map(PathBuf::from);
    let location = decide(&home, env.as_deref(), read_pointer(&home));
    if location.kind == Kind::Chosen {
        std::env::set_var(DATA_DIR_VAR, &location.folder);
    }
    set_current(location.clone());
    Some(location)
}

pub fn read_pointer(home: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(home.join(POINTER_FILE)).ok()?;
    let text = decode_text(&bytes)?;
    let path = PathBuf::from(text.trim());
    path.is_absolute().then_some(path)
}

fn write_pointer(home: &Path, chosen: Option<&Path>) -> io::Result<()> {
    let path = home.join(POINTER_FILE);
    let Some(chosen) = chosen else {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    };
    std::fs::create_dir_all(home)?;
    let temporary = home.join(format!("{POINTER_FILE}.tmp"));
    std::fs::write(&temporary, encode_text(&chosen.to_string_lossy()))?;
    std::fs::rename(&temporary, &path)
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    if let Some(wide) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = wide.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        return String::from_utf16(&units).ok();
    }
    String::from_utf8(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes).to_vec()).ok()
}

fn encode_text(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()
    } else {
        text.as_bytes().to_vec()
    }
}

pub fn schedule(folder: &Path, pending: &Pending) -> io::Result<()> {
    std::fs::write(folder.join(PENDING_FILE), serde_json::to_vec(pending)?)
}

pub fn scheduled(folder: &Path) -> Option<Pending> {
    serde_json::from_slice(&std::fs::read(folder.join(PENDING_FILE)).ok()?).ok()
}

fn take_scheduled(folder: &Path) -> Option<Pending> {
    let pending = scheduled(folder);
    let _ = std::fs::remove_file(folder.join(PENDING_FILE));
    pending
}

pub fn finish_scheduled_before_start() {
    let Some(location) = current() else { return };
    let Some(pending) = take_scheduled(&location.folder) else { return };
    match pending {
        Pending::Move { to } => match move_folder(&location, &to, switch_to) {
            Ok(moved) => tracing::info!("moved Commy's data to {}", moved.folder.display()),
            Err(why) => {
                tracing::error!("could not move Commy's data to {}: {why}", to.display());
                tell_move_failed(&location.folder, &why);
            }
        },
        Pending::ClearWebCache => {
            if let Some(engine) = location.webview_engine_dir() {
                remove_cache_dirs(&engine);
            }
        }
        Pending::ClearEverything => clear_everything(&location),
    }
}

#[cfg(desktop)]
fn tell_move_failed(folder: &Path, why: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Commy could not move your data")
        .set_description(format!("{why}\n\nNothing was lost. Commy keeps using {}.", folder.display()))
        .show();
}

#[cfg(mobile)]
fn tell_move_failed(_: &Path, _: &str) {}

fn is_commy_entry(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else { return false };
    COMMY_ENTRIES.contains(&name)
        || crate::account_folder::account_entries().iter().any(|entry| entry == name)
        || COMMY_ENTRY_PATTERNS.iter().any(|(start, end)| name.starts_with(start) && name.ends_with(end))
}

fn top_level(folder: &Path) -> Vec<std::ffi::OsString> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut names: Vec<_> = entries.flatten().map(|entry| entry.file_name()).collect();
    names.sort();
    names
}

fn stays_put(name: &OsStr) -> bool {
    STAYS_PUT.iter().any(|kept| OsStr::new(kept) == name)
}

fn carried_names(location: &Location) -> Vec<std::ffi::OsString> {
    let at_home = location.at_home();
    top_level(&location.folder)
        .into_iter()
        .filter(|name| {
            if at_home {
                is_commy_entry(name)
            } else {
                !stays_put(name) && !(cfg!(windows) && name.as_os_str() == crate::instance::WEBVIEW_DIR)
            }
        })
        .collect()
}

fn moved_location(from: &Location, to: &Path) -> Location {
    let kind = if same_path(to, &from.home) { Kind::Default } else { Kind::Chosen };
    Location { home: from.home.clone(), folder: to.to_path_buf(), kind }
}

pub fn target_for(picked: &Path, home: &Path) -> PathBuf {
    if same_path(picked, home) {
        return home.to_path_buf();
    }
    let dedicated = picked.parent().is_some() && (!picked.exists() || is_empty_dir(picked));
    if dedicated {
        picked.to_path_buf()
    } else {
        picked.join(NEW_FOLDER_NAME)
    }
}

fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

pub fn check_target(from: &Location, to: &Path) -> Result<(), String> {
    if from.kind == Kind::Overridden {
        return Err(format!("This copy of Commy uses a folder set with {DATA_DIR_VAR}, so it cannot be moved from here."));
    }
    if !usable_folder(to) {
        return Err("Choose a folder inside a drive, not the drive itself.".into());
    }
    if same_path(to, &from.folder) {
        return Err("Commy already keeps its data there.".into());
    }
    if inside(to, &from.folder) {
        return Err("Choose a folder outside the one Commy uses now.".into());
    }
    if inside(&from.folder, to) {
        return Err("Choose a folder that does not hold the one Commy uses now.".into());
    }
    if !to.exists() {
        return Ok(());
    }
    if !to.is_dir() {
        return Err("That is a file, not a folder.".into());
    }
    let names = top_level(to);
    let clear = if same_path(to, &from.home) {
        !names.iter().any(|name| is_commy_entry(name))
    } else {
        names.iter().all(|name| stays_put(name))
    };
    if clear {
        Ok(())
    } else {
        Err("That folder already has files in it. Choose an empty folder.".into())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MovePlan {
    pub target: String,
    pub to_default: bool,
    pub bytes: u64,
    pub free: Option<u64>,
}

pub fn plan_move(from: &Location, picked: &Path) -> Result<MovePlan, String> {
    let to = target_for(picked, &from.home);
    check_target(from, &to)?;
    let bytes = bytes_to_carry(from);
    let free = free_space(&to);
    if let Some(free) = free.filter(|free| bytes.saturating_add(SPARE_BYTES) > *free) {
        return Err(format!("That drive has {} free, and your data needs {}.", human_size(free), human_size(bytes)));
    }
    check_writable(&to)?;
    Ok(MovePlan { target: to.to_string_lossy().into_owned(), to_default: same_path(&to, &from.home), bytes, free })
}

fn check_writable(to: &Path) -> Result<(), String> {
    let created = !to.exists();
    let probe = to.join(PROBE_FILE);
    let written = std::fs::create_dir_all(to).and_then(|_| std::fs::write(&probe, b"commy"));
    let _ = std::fs::remove_file(&probe);
    if created {
        let _ = std::fs::remove_dir(to);
    }
    written.map_err(|e| format!("Commy cannot save files there ({e})."))
}

fn bytes_to_carry(from: &Location) -> u64 {
    let carried: u64 = carried_names(from).iter().map(|name| tree_size(&from.folder.join(name), &|_| false)).sum();
    let web = from.webview_engine_dir().map_or(0, |engine| tree_size(&engine, &is_cache_dir));
    carried + web
}

fn web_part(from: &Location, to: &Location) -> Option<(PathBuf, PathBuf)> {
    let source = from.webview_engine_dir()?;
    let target = to.webview_engine_dir()?;
    (source.is_dir() && !same_path(&source, &target)).then_some((source, target))
}

pub fn move_folder(from: &Location, to: &Path, switch: impl FnOnce(&Location, &Location)) -> Result<Location, String> {
    check_target(from, to)?;
    let carried = carried_names(from);
    let target = moved_location(from, to);
    let web = web_part(from, &target);
    let needed = bytes_to_carry(from);
    if let Some(free) = free_space(to).filter(|free| needed > *free) {
        return Err(format!("The drive has {} free, and your data needs {}.", human_size(free), human_size(needed)));
    }
    let created = !to.exists();
    let copied = copy_everything(from, to, &carried, web.as_ref());
    if let Err(why) = copied {
        discard_copy(to, created, &carried, web.as_ref());
        return Err(why);
    }
    switch(&target, from);
    remove_old_copy(from, &carried, web.as_ref());
    Ok(target)
}

fn copy_everything(
    from: &Location,
    to: &Path,
    carried: &[std::ffi::OsString],
    web: Option<&(PathBuf, PathBuf)>,
) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("Could not create {} ({e}).", to.display()))?;
    for name in carried {
        let (source, target) = (from.folder.join(name), to.join(name));
        copy_and_verify(&source, &target, &|_| false)?;
    }
    if let Some((source, target)) = web {
        if target.exists() {
            std::fs::remove_dir_all(target).map_err(|e| format!("Could not replace {} ({e}).", target.display()))?;
        }
        copy_and_verify(source, target, &is_cache_dir)?;
    }
    crate::local_key::carry(&from.folder, to)?;
    let chosen = (!same_path(to, &from.home)).then_some(to);
    write_pointer(&from.home, chosen).map_err(|e| format!("Could not remember the new folder ({e})."))
}

fn copy_and_verify(source: &Path, target: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    copy_tree(source, target, skip).map_err(|e| format!("Could not copy {} ({e}).", source.display()))?;
    match verify_tree(source, target, skip) {
        Ok(None) => Ok(()),
        Ok(Some(different)) => Err(format!("{} did not copy correctly.", different.display())),
        Err(e) => Err(format!("Could not check the copy of {} ({e}).", source.display())),
    }
}

fn discard_copy(to: &Path, created: bool, carried: &[std::ffi::OsString], web: Option<&(PathBuf, PathBuf)>) {
    if let Some((_, target)) = web {
        let _ = std::fs::remove_dir_all(target);
    }
    if created {
        let _ = std::fs::remove_dir_all(to);
        return;
    }
    for name in carried {
        remove_entry(&to.join(name));
    }
}

fn switch_to(target: &Location, from: &Location) {
    match target.kind {
        Kind::Chosen => std::env::set_var(DATA_DIR_VAR, &target.folder),
        _ if crate::instance::TEST_BUILD => std::env::set_var(DATA_DIR_VAR, &target.folder),
        _ => std::env::remove_var(DATA_DIR_VAR),
    }
    let old_webview = from.folder.join(crate::instance::WEBVIEW_DIR);
    if std::env::var_os(WEBVIEW_FOLDER_VAR).is_some_and(|set| same_path(Path::new(&set), &old_webview)) {
        std::env::remove_var(WEBVIEW_FOLDER_VAR);
    }
    set_current(target.clone());
    #[cfg(desktop)]
    crate::single_instance::hold_instead(&target.folder);
}

fn remove_old_copy(from: &Location, carried: &[std::ffi::OsString], web: Option<&(PathBuf, PathBuf)>) {
    for name in carried {
        remove_entry(&from.folder.join(name));
    }
    if let Some((source, _)) = web {
        remove_entry(source);
    }
    if !from.at_home() {
        remove_entry(&from.folder.join(crate::instance::WEBVIEW_DIR));
        remove_entry(&from.folder.join(RUNNING_LOCK_FILE));
        let _ = std::fs::remove_dir(&from.folder);
    }
}

fn remove_entry(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return };
    let removed = if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    if let Err(e) = removed {
        tracing::warn!("could not remove {}: {e}", path.display());
    }
}

fn clear_everything(location: &Location) {
    let at_home = location.at_home();
    for name in top_level(&location.folder) {
        let wanted = if at_home {
            !stays_put(&name) && name.as_os_str() != crate::instance::TEST_BUILD_DIR
        } else {
            is_commy_entry(&name) || name.as_os_str() == crate::instance::WEBVIEW_DIR
        };
        if wanted {
            remove_entry(&location.folder.join(&name));
        }
    }
    if let Some(engine) = location.webview_engine_dir() {
        remove_entry(&engine);
    }
    crate::local_key::forget(&location.folder);
    crate::account_folder::show_window_on_next_start(&location.folder);
    tracing::info!("cleared Commy's data in {}", location.folder.display());
}

pub fn clear_cache_now(base: &Path) -> u64 {
    let mut freed = 0;
    let mut folders = crate::account_folder::folders(base);
    folders.push(base.to_path_buf());
    for folder in folders {
        let pictures = folder.join(crate::profile_images::PROFILE_IMAGE_DIR);
        freed += tree_size(&pictures, &|_| false);
        remove_entry(&pictures);
    }
    let previous_log = base.join(crate::debug_log::LOG_FOLDER).join(crate::debug_log::PREVIOUS_LOG_FILE);
    freed += tree_size(&previous_log, &|_| false);
    remove_entry(&previous_log);
    freed
}

fn remove_cache_dirs(engine: &Path) {
    let Ok(entries) = std::fs::read_dir(engine) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if is_cache_dir(&path) {
            remove_entry(&path);
        } else {
            remove_cache_dirs(&path);
        }
    }
}

fn is_cache_dir(path: &Path) -> bool {
    path.file_name().and_then(OsStr::to_str).is_some_and(|name| WEBVIEW_CACHE_DIRS.contains(&name)) && path.is_dir()
}

fn copy_tree(source: &Path, target: &Path, skip: &dyn Fn(&Path) -> bool) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(source)?;
    if meta.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let path = entry?.path();
            if skip(&path) {
                continue;
            }
            if let Some(name) = path.file_name() {
                copy_tree(&path, &target.join(name), skip)?;
            }
        }
    } else if meta.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, target)?;
    }
    Ok(())
}

fn verify_tree(source: &Path, target: &Path, skip: &dyn Fn(&Path) -> bool) -> io::Result<Option<PathBuf>> {
    let meta = std::fs::symlink_metadata(source)?;
    if meta.is_dir() {
        for entry in std::fs::read_dir(source)? {
            let path = entry?.path();
            if skip(&path) {
                continue;
            }
            if let Some(name) = path.file_name() {
                if let Some(different) = verify_tree(&path, &target.join(name), skip)? {
                    return Ok(Some(different));
                }
            }
        }
    } else if meta.is_file() && !same_contents(source, target)? {
        return Ok(Some(source.to_path_buf()));
    }
    Ok(None)
}

fn same_contents(a: &Path, b: &Path) -> io::Result<bool> {
    let Ok(mut second) = File::open(b) else { return Ok(false) };
    let mut first = File::open(a)?;
    if first.metadata()?.len() != second.metadata()?.len() {
        return Ok(false);
    }
    let (mut left, mut right) = (vec![0u8; COMPARE_CHUNK], vec![0u8; COMPARE_CHUNK]);
    loop {
        let read = fill(&mut first, &mut left)?;
        if read != fill(&mut second, &mut right)? || left[..read] != right[..read] {
            return Ok(false);
        }
        if read == 0 {
            return Ok(true);
        }
    }
}

fn fill(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

pub fn tree_size(path: &Path, skip: &dyn Fn(&Path) -> bool) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries.flatten().map(|entry| entry.path()).filter(|child| !skip(child)).map(|child| tree_size(&child, skip)).sum()
}

fn cache_size(path: &Path) -> u64 {
    if is_cache_dir(path) {
        return tree_size(path, &|_| false);
    }
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries.flatten().filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir())).map(|entry| cache_size(&entry.path())).sum()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsagePart {
    pub id: &'static str,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub total: u64,
    pub free: Option<u64>,
    pub parts: Vec<UsagePart>,
}

pub fn usage(location: &Location, account: Option<&Path>) -> Usage {
    let folder = &location.folder;
    let at_home = location.at_home();
    let mut total: u64 = top_level(folder)
        .iter()
        .filter(|name| !(at_home && name.as_os_str() == crate::instance::TEST_BUILD_DIR))
        .map(|name| tree_size(&folder.join(name), &|_| false))
        .sum();
    let engine = location.webview_engine_dir().filter(|engine| !inside(engine, folder));
    total += engine.as_deref().map_or(0, |engine| tree_size(engine, &|_| false));
    let web_cache = location.webview_engine_dir().map_or(0, |engine| cache_size(&engine));
    let account = account.filter(|dir| dir.is_dir());
    let size_in_account = |name: &str| account.map_or(0, |dir| tree_size(&dir.join(name), &|_| false));
    let shared_files = size_in_account(crate::SHARED_FILES_DIR);
    let pictures = size_in_account(crate::profile_images::PROFILE_IMAGE_DIR);
    let messages = account.map_or(0, |dir| tree_size(dir, &|_| false)).saturating_sub(shared_files + pictures);
    let other_accounts = crate::account_folder::folders(folder)
        .iter()
        .filter(|dir| !account.is_some_and(|mine| same_path(dir, mine)))
        .map(|dir| tree_size(dir, &|_| false))
        .sum();
    let voices = tree_size(&folder.join(crate::voice_pack::VOICES_DIR), &|_| false);
    let logs = tree_size(&folder.join(crate::debug_log::LOG_FOLDER), &|_| false);
    let mut parts = vec![
        UsagePart { id: "messages", bytes: messages },
        UsagePart { id: "shared_files", bytes: shared_files },
        UsagePart { id: "pictures", bytes: pictures },
        UsagePart { id: "voices", bytes: voices },
        UsagePart { id: "web_cache", bytes: web_cache },
        UsagePart { id: "logs", bytes: logs },
        UsagePart { id: "other_accounts", bytes: other_accounts },
    ];
    let known: u64 = parts.iter().map(|part| part.bytes).sum();
    parts.push(UsagePart { id: "other", bytes: total.saturating_sub(known) });
    Usage { total: total.max(known), free: free_space(folder), parts }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn comparable(path: &Path) -> Vec<String> {
    path.components()
        .filter(|part| !matches!(part, Component::CurDir))
        .map(|part| {
            let text = part.as_os_str().to_string_lossy();
            if cfg!(windows) {
                text.trim_end_matches(['\\', '/']).to_lowercase()
            } else {
                text.into_owned()
            }
        })
        .filter(|text| !text.is_empty() || !cfg!(windows))
        .collect()
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    comparable(a) == comparable(b)
}

pub fn inside(child: &Path, parent: &Path) -> bool {
    let (child, parent) = (comparable(child), comparable(parent));
    child.len() > parent.len() && child.starts_with(&parent)
}

fn nearest_existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|dir| dir.exists())
}

#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let dir = nearest_existing(path)?;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available: u64 = 0;
    // SAFETY: wide is a NUL-terminated UTF-16 path that outlives the call, available is a valid u64 out pointer, and the two optional outputs are null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(available)
}

#[cfg(unix)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let dir = nearest_existing(path)?;
    let text = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is a plain C struct for which all zero bytes is a valid value.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: text is a NUL-terminated path that outlives the call and stats is a valid, writable statvfs.
    let ok = unsafe { libc::statvfs(text.as_ptr(), &mut stats) } == 0;
    #[allow(clippy::unnecessary_cast)]
    ok.then(|| stats.f_bavail as u64 * stats.f_frsize as u64)
}

const NO_FOLDER: &str = "This device has no data folder";
pub const CLEAR_ALL_WORD: &str = "DELETE";

#[derive(Debug, Clone, Serialize)]
pub struct DataFolderView {
    pub folder: String,
    pub standard_folder: String,
    pub kind: Kind,
    pub can_move: bool,
    pub scheduled: Option<Pending>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClearedCache {
    pub freed: u64,
    pub on_restart: u64,
}

fn location_of(app: &App) -> Option<Location> {
    current().or_else(|| {
        let base = app.base_dir()?.clone();
        Some(Location { home: base.clone(), folder: base, kind: Kind::Default })
    })
}

#[tauri::command]
pub fn data_folder(app: tauri::State<'_, App>) -> Result<DataFolderView, String> {
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    Ok(DataFolderView {
        folder: location.folder.to_string_lossy().into_owned(),
        standard_folder: location.home.to_string_lossy().into_owned(),
        kind: location.kind,
        can_move: cfg!(desktop) && location.kind != Kind::Overridden,
        scheduled: scheduled(&location.folder),
    })
}

#[tauri::command]
pub fn open_data_folder(app: tauri::State<'_, App>) -> Result<(), String> {
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    #[cfg(desktop)]
    return open::that_detached(&location.folder).map_err(|e| e.to_string());
    #[cfg(mobile)]
    {
        let _ = location;
        Err("Commy keeps its data in the app's own storage on this device".into())
    }
}

#[tauri::command]
pub async fn storage_usage(app: tauri::State<'_, App>) -> Result<Usage, String> {
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    let account = app.data_dir().cloned();
    tauri::async_runtime::spawn_blocking(move || usage(&location, account.as_deref())).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pick_data_folder() -> Result<Option<String>, String> {
    #[cfg(desktop)]
    {
        let picked = rfd::AsyncFileDialog::new().set_title("Choose where Commy keeps its data").pick_folder().await;
        Ok(picked.map(|folder| folder.path().to_string_lossy().into_owned()))
    }
    #[cfg(mobile)]
    Err("Commy keeps its data in the app's own storage on this device".into())
}

#[tauri::command]
pub async fn plan_data_folder_move(app: tauri::State<'_, App>, folder: String) -> Result<MovePlan, String> {
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    if !cfg!(desktop) {
        return Err("Commy keeps its data in the app's own storage on this device".into());
    }
    let picked = PathBuf::from(folder.trim());
    tauri::async_runtime::spawn_blocking(move || plan_move(&location, &picked)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn move_data_folder(app: tauri::State<'_, App>, handle: tauri::AppHandle, target: String) -> Result<(), String> {
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    let to = PathBuf::from(target.trim());
    if !cfg!(desktop) {
        return Err("Commy keeps its data in the app's own storage on this device".into());
    }
    check_target(&location, &to)?;
    schedule(&location.folder, &Pending::Move { to }).map_err(|e| format!("Could not get the move ready ({e})"))?;
    crate::account_folder::show_window_on_next_start(&location.folder);
    crate::relaunch::restart(&handle)
}

#[tauri::command]
pub async fn clear_cache(app: tauri::State<'_, App>) -> Result<ClearedCache, String> {
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    let base = app.base_dir().cloned().ok_or(NO_FOLDER)?;
    let engine = location.webview_engine_dir();
    let (freed, on_restart) = tauri::async_runtime::spawn_blocking(move || {
        let on_restart = engine.as_deref().map_or(0, cache_size);
        (clear_cache_now(&base), on_restart)
    })
    .await
    .map_err(|e| e.to_string())?;
    if on_restart > 0 && scheduled(&location.folder).is_none() {
        schedule(&location.folder, &Pending::ClearWebCache).map_err(|e| e.to_string())?;
    }
    Ok(ClearedCache { freed, on_restart })
}

#[tauri::command]
pub async fn clear_all_data(app: tauri::State<'_, App>, handle: tauri::AppHandle, confirm: String) -> Result<(), String> {
    if confirm.trim() != CLEAR_ALL_WORD {
        return Err(format!("Type {CLEAR_ALL_WORD} to confirm"));
    }
    let location = location_of(&app).ok_or(NO_FOLDER)?;
    schedule(&location.folder, &Pending::ClearEverything).map_err(|e| format!("Could not get ready to clear ({e})"))?;
    crate::account::sign_out_everyone_here(&app).await;
    crate::relaunch::restart(&handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("commy-data-location-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn home_location(home: &Path) -> Location {
        Location { home: home.into(), folder: home.into(), kind: Kind::Default }
    }

    fn fill_home(home: &Path) {
        write(&home.join("account.json"), b"account");
        write(&home.join("device-data.key"), b"0011223344556677889900112233445566778899001122334455667788990011");
        write(&home.join("accounts/abc/servers/one.json"), b"server one");
        write(&home.join("accounts/abc/dms.json"), b"dms");
        write(&home.join("accounts/abc/profile-images/x.img"), b"picture");
        write(&home.join("logs/commy.log"), b"log");
        write(&home.join("logs/commy.previous.log"), b"older log");
        write(&home.join("entry-key-abc.json"), b"salvaged");
        write(&home.join("voices/kristin.onnx"), &[7u8; 4096]);
        write(&home.join("test-build/account.json"), b"test build account");
        write(&home.join("localstorage/web.sqlite"), b"webkit");
        write(&home.join("running.lock"), b"");
        write(&home.join(NOTIFICATION_ICON_FILE), b"png");
    }

    #[test]
    fn the_standard_folder_is_used_until_one_is_chosen() {
        let home = Path::new("/data/home");
        assert_eq!(decide(home, None, None), home_location(home));
        let chosen = decide(home, None, Some(PathBuf::from("/games/Commy")));
        assert_eq!(chosen.kind, Kind::Chosen);
        assert_eq!(chosen.folder, PathBuf::from("/games/Commy"));
    }

    #[test]
    fn a_folder_set_by_hand_wins_and_a_restart_keeps_the_chosen_one() {
        let home = Path::new("/data/home");
        let bench = decide(home, Some(Path::new("/bench/A")), Some(PathBuf::from("/games/Commy")));
        assert_eq!((bench.kind, bench.folder), (Kind::Overridden, PathBuf::from("/bench/A")));
        let restarted = decide(home, Some(Path::new("/games/Commy")), Some(PathBuf::from("/games/Commy")));
        assert_eq!(restarted.kind, Kind::Chosen);
        let test_build = decide(home, Some(home), None);
        assert_eq!(test_build.kind, Kind::Default);
    }

    #[test]
    fn a_pointer_to_a_drive_root_or_a_relative_path_is_ignored() {
        let home = Path::new("/data/home");
        assert_eq!(decide(home, None, Some(PathBuf::from("/"))).kind, Kind::Default);
        assert_eq!(decide(home, None, Some(PathBuf::from("games"))).kind, Kind::Default);
    }

    #[test]
    fn pointers_written_by_the_installer_or_the_app_read_back() {
        let home = temp("pointer");
        let target = std::env::temp_dir().join("Commy Daten");
        let wide: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(format!("{}\r\n", target.display()).encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        std::fs::write(home.join(POINTER_FILE), wide).unwrap();
        assert_eq!(read_pointer(&home), Some(target.clone()));
        write_pointer(&home, Some(&target)).unwrap();
        assert_eq!(read_pointer(&home), Some(target));
        write_pointer(&home, None).unwrap();
        assert_eq!(read_pointer(&home), None);
        write_pointer(&home, None).unwrap();
    }

    #[test]
    fn a_busy_folder_gets_a_commy_subfolder() {
        let root = temp("target");
        let home = root.join("home");
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let busy = root.join("busy");
        write(&busy.join("notes.txt"), b"mine");
        assert_eq!(target_for(&empty, &home), empty);
        assert_eq!(target_for(&root.join("new"), &home), root.join("new"));
        assert_eq!(target_for(&busy, &home), busy.join(NEW_FOLDER_NAME));
        assert_eq!(target_for(&home, &home), home);
        assert_eq!(target_for(Path::new("/"), &home), Path::new("/").join(NEW_FOLDER_NAME));
    }

    #[test]
    fn targets_that_would_mix_or_nest_data_are_refused() {
        let root = temp("refuse");
        let home = root.join("home");
        fill_home(&home);
        let from = home_location(&home);
        assert!(check_target(&from, &home).is_err());
        assert!(check_target(&from, &home.join("accounts")).is_err());
        assert!(check_target(&from, &root).is_err());
        let busy = root.join("busy");
        write(&busy.join("notes.txt"), b"mine");
        assert!(check_target(&from, &busy).is_err());
        assert!(check_target(&from, &root.join("fresh")).is_ok());
        let bench = Location { home: home.clone(), folder: root.join("bench"), kind: Kind::Overridden };
        assert!(check_target(&bench, &root.join("fresh")).is_err());
    }

    #[test]
    fn moving_copies_checks_and_then_removes_only_commy_files() {
        let root = temp("move-out");
        let home = root.join("home");
        fill_home(&home);
        let to = root.join("games").join("Commy");
        let moved = move_folder(&home_location(&home), &to, |_, _| {}).unwrap();
        assert_eq!(moved.kind, Kind::Chosen);
        assert_eq!(read_pointer(&home), Some(to.clone()));
        assert_eq!(std::fs::read(to.join("accounts/abc/servers/one.json")).unwrap(), b"server one");
        assert_eq!(std::fs::read(to.join("entry-key-abc.json")).unwrap(), b"salvaged");
        assert_eq!(std::fs::read(to.join("voices/kristin.onnx")).unwrap(), vec![7u8; 4096]);
        assert!(!home.join("accounts").exists());
        assert!(!home.join("account.json").exists());
        assert!(home.join("test-build/account.json").exists(), "the test build keeps its own folder");
        assert!(home.join("localstorage/web.sqlite").exists(), "the web view's own files stay where it reads them");
        assert!(home.join(NOTIFICATION_ICON_FILE).exists());
        assert!(!to.join("test-build").exists());
        assert!(!to.join("localstorage").exists());
        assert!(!to.join(POINTER_FILE).exists());
    }

    #[test]
    fn moving_back_to_the_standard_folder_forgets_the_choice() {
        let root = temp("move-back");
        let home = root.join("home");
        write(&home.join(NOTIFICATION_ICON_FILE), b"png");
        let chosen = root.join("chosen");
        write(&chosen.join("account.json"), b"account");
        write(&chosen.join("accounts/abc/dms.json"), b"dms");
        write(&chosen.join("running.lock"), b"");
        write_pointer(&home, Some(&chosen)).unwrap();
        let from = Location { home: home.clone(), folder: chosen.clone(), kind: Kind::Chosen };
        let moved = move_folder(&from, &home, |_, _| {}).unwrap();
        assert_eq!(moved.kind, Kind::Default);
        assert_eq!(read_pointer(&home), None);
        assert_eq!(std::fs::read(home.join("accounts/abc/dms.json")).unwrap(), b"dms");
        assert!(!chosen.exists(), "the emptied folder is removed");
    }

    #[test]
    fn a_copy_that_cannot_finish_is_discarded_and_the_original_kept() {
        let root = temp("move-fail");
        let home = root.join("home");
        fill_home(&home);
        let blocked = root.join("blocked");
        write(&blocked, b"a file where the folder should go");
        let to = blocked.join("Commy");
        assert!(move_folder(&home_location(&home), &to, |_, _| panic!("a failed copy never switches")).is_err());
        assert!(home.join("accounts/abc/dms.json").exists());
        assert_eq!(read_pointer(&home), None);
    }

    #[test]
    fn a_different_file_fails_the_check() {
        let root = temp("verify");
        write(&root.join("a/one"), b"same");
        write(&root.join("b/one"), b"same");
        assert_eq!(verify_tree(&root.join("a"), &root.join("b"), &|_| false).unwrap(), None);
        write(&root.join("b/one"), b"diff");
        assert_eq!(verify_tree(&root.join("a"), &root.join("b"), &|_| false).unwrap(), Some(root.join("a/one")));
        std::fs::remove_file(root.join("b/one")).unwrap();
        assert!(verify_tree(&root.join("a"), &root.join("b"), &|_| false).unwrap().is_some());
    }

    #[test]
    fn clearing_everything_keeps_the_choice_and_the_test_build() {
        let root = temp("clear-all");
        let home = root.join("home");
        fill_home(&home);
        write_pointer(&home, Some(&root.join("elsewhere"))).unwrap();
        clear_everything(&home_location(&home));
        let mut left: Vec<_> = top_level(&home).into_iter().map(|n| n.to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [POINTER_FILE, NOTIFICATION_ICON_FILE, "running.lock", "show-window-on-start", "test-build"]);
    }

    #[test]
    fn clearing_a_chosen_folder_leaves_files_commy_did_not_make() {
        let root = temp("clear-chosen");
        let chosen = root.join("chosen");
        write(&chosen.join("account.json"), b"account");
        write(&chosen.join("accounts/abc/dms.json"), b"dms");
        write(&chosen.join("my notes.txt"), b"mine");
        clear_everything(&Location { home: root.join("home"), folder: chosen.clone(), kind: Kind::Chosen });
        assert!(chosen.join("my notes.txt").exists());
        assert!(!chosen.join("accounts").exists());
        assert!(!chosen.join("account.json").exists());
    }

    #[test]
    fn clearing_the_cache_keeps_messages_and_the_current_log() {
        let root = temp("clear-cache");
        fill_home(&root);
        assert_eq!(clear_cache_now(&root), 16);
        assert!(!root.join("accounts/abc/profile-images").exists());
        assert!(!root.join("logs/commy.previous.log").exists());
        assert!(root.join("logs/commy.log").exists());
        assert!(root.join("accounts/abc/dms.json").exists());
    }

    #[test]
    fn web_caches_are_found_at_any_depth() {
        let root = temp("web-cache");
        write(&root.join("EBWebView/Default/Cache/Cache_Data/f_1"), &[1u8; 10]);
        write(&root.join("EBWebView/GrShaderCache/data_0"), &[1u8; 5]);
        write(&root.join("EBWebView/Default/Local Storage/leveldb/000003.log"), &[1u8; 3]);
        assert_eq!(cache_size(&root.join("EBWebView")), 15);
        remove_cache_dirs(&root.join("EBWebView"));
        assert_eq!(tree_size(&root.join("EBWebView"), &|_| false), 3);
    }

    #[test]
    fn usage_splits_the_folder_into_parts_that_add_up() {
        let root = temp("usage");
        fill_home(&root);
        write(&root.join("accounts/abc/shared-files/blobs.db"), &[1u8; 100]);
        write(&root.join("accounts/other/dms.json"), &[1u8; 50]);
        let location = home_location(&root);
        let found = usage(&location, Some(&root.join("accounts/abc")));
        let part = |id: &str| found.parts.iter().find(|p| p.id == id).unwrap().bytes;
        assert_eq!(part("shared_files"), 100);
        assert_eq!(part("pictures"), 7);
        assert_eq!(part("messages"), 13);
        assert_eq!(part("other_accounts"), 50);
        assert_eq!(part("voices"), 4096);
        assert_eq!(part("logs"), 12);
        assert_eq!(found.parts.iter().map(|p| p.bytes).sum::<u64>(), found.total);
        assert!(found.free.is_some());
    }

    #[test]
    fn sizes_read_like_people_write_them() {
        assert_eq!(human_size(512), "512 bytes");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn paths_compare_by_their_parts() {
        assert!(same_path(Path::new("/a/b/"), Path::new("/a/b")));
        assert!(inside(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(!inside(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!inside(Path::new("/a/b"), Path::new("/a/b")));
    }

    #[test]
    fn pending_work_round_trips() {
        let dir = temp("pending");
        let work = Pending::Move { to: PathBuf::from("/games/Commy") };
        schedule(&dir, &work).unwrap();
        assert_eq!(scheduled(&dir), Some(work.clone()));
        assert_eq!(take_scheduled(&dir), Some(work));
        assert_eq!(scheduled(&dir), None);
    }
}
