use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::data_location::RUNNING_LOCK_FILE as LOCK_FILE;

const WAIT_FOR_RUNNING_COPY: Duration = Duration::from_secs(20);
const RETRY_EVERY: Duration = Duration::from_millis(250);

static HELD: Mutex<Option<File>> = Mutex::new(None);

#[derive(Debug)]
enum Outcome {
    First(Option<File>),
    HandedOff,
    GaveUp,
}

enum Lock {
    Held(File),
    Busy,
    Unavailable,
}

pub fn data_dir() -> Option<PathBuf> {
    crate::invite_link::default_data_dir().or_else(linux_data_dir)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    Some(base.join(crate::toast::APP_ID))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn linux_data_dir() -> Option<PathBuf> {
    None
}

pub fn first_copy(dir: &Path, request: &str) -> bool {
    match claim(dir, || hand_off(dir, request), WAIT_FOR_RUNNING_COPY, RETRY_EVERY) {
        Outcome::First(lock) => {
            if let Some(lock) = lock {
                *HELD.lock().unwrap() = Some(lock);
            }
            true
        }
        Outcome::HandedOff => false,
        Outcome::GaveUp => {
            tracing::warn!("Commy is already running for {} and did not answer; leaving it be", dir.display());
            false
        }
    }
}

pub fn hold_instead(dir: &Path) {
    match try_hold(dir) {
        Lock::Held(file) => *HELD.lock().unwrap() = Some(file),
        Lock::Busy => tracing::warn!("another copy of Commy holds {}; keeping the old lock", dir.display()),
        Lock::Unavailable => *HELD.lock().unwrap() = None,
    }
}

fn hand_off(dir: &Path, request: &str) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
        AllowSetForegroundWindow(ASFW_ANY);
    }
    crate::invite_link::hand_to_running_instance(dir, request)
}

fn claim(dir: &Path, mut hand_off: impl FnMut() -> bool, wait: Duration, every: Duration) -> Outcome {
    let deadline = Instant::now() + wait;
    loop {
        match try_hold(dir) {
            Lock::Held(file) => return Outcome::First(Some(file)),
            Lock::Unavailable => return Outcome::First(None),
            Lock::Busy => {}
        }
        if hand_off() {
            return Outcome::HandedOff;
        }
        if Instant::now() >= deadline {
            return Outcome::GaveUp;
        }
        std::thread::sleep(every);
    }
}

fn try_hold(dir: &Path) -> Lock {
    let opened = std::fs::create_dir_all(dir).and_then(|_| {
        OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(LOCK_FILE))
    });
    let file = match opened {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!("could not open the running-copy lock in {}: {e}", dir.display());
            return Lock::Unavailable;
        }
    };
    match file.try_lock() {
        Ok(()) => Lock::Held(file),
        Err(TryLockError::WouldBlock) => Lock::Busy,
        Err(TryLockError::Error(e)) => {
            tracing::warn!("could not take the running-copy lock in {}: {e}", dir.display());
            Lock::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("commy-single-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const QUICK: Duration = Duration::from_millis(10);

    #[test]
    fn the_first_copy_holds_the_lock_and_a_second_hands_off() {
        let dir = dir("handoff");
        let Outcome::First(Some(_running)) = claim(&dir, || panic!("nobody to hand off to"), QUICK, QUICK) else {
            panic!("the first copy should start");
        };
        let asked = Cell::new(0);
        let second = claim(&dir, || { asked.set(asked.get() + 1); true }, QUICK, QUICK);
        assert!(matches!(second, Outcome::HandedOff));
        assert_eq!(asked.get(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_that_never_answers_still_keeps_the_second_out() {
        let dir = dir("silent");
        let Outcome::First(Some(_running)) = claim(&dir, || false, QUICK, QUICK) else { panic!() };
        assert!(matches!(claim(&dir, || false, Duration::from_millis(50), QUICK), Outcome::GaveUp));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restart_takes_over_once_the_old_copy_exits() {
        let dir = dir("restart");
        let Outcome::First(Some(old)) = claim(&dir, || false, QUICK, QUICK) else { panic!() };
        let exiting = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(old);
        });
        let new = claim(&dir, || false, Duration::from_secs(10), QUICK);
        exiting.join().unwrap();
        assert!(matches!(new, Outcome::First(Some(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_with_their_own_data_folders_run_side_by_side() {
        let a = dir("bench-a");
        let b = dir("bench-b");
        let Outcome::First(Some(_first)) = claim(&a, || false, QUICK, QUICK) else { panic!() };
        assert!(matches!(claim(&b, || false, QUICK, QUICK), Outcome::First(Some(_))));
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
