use crate::lock_or_recover::LockOrRecover;
use crate::state::App;
use std::future::Future;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

static TASKS: Mutex<Vec<tokio::task::AbortHandle>> = Mutex::new(Vec::new());
static CLOSING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RELOAD_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn spawn<F>(task: F) -> tauri::async_runtime::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handle = tauri::async_runtime::spawn(task);
    let mut tasks = TASKS.lock_or_recover();
    tasks.retain(|t| !t.is_finished());
    tasks.push(handle.inner().abort_handle());
    handle
}

fn stop_tasks() -> usize {
    let tasks = std::mem::take(&mut *TASKS.lock_or_recover());
    let running = tasks.iter().filter(|t| !t.is_finished()).count();
    for task in tasks {
        task.abort();
    }
    running
}

pub fn switches_in_place() -> bool {
    cfg!(target_os = "ios") || std::env::var_os("COMMY_SWITCH_IN_PLACE").is_some()
}

pub async fn close(handle: &AppHandle) {
    let _closing = CLOSING.lock().await;
    let app = handle.state::<App>();
    if app.data_dir().is_none() {
        return;
    }
    tracing::info!("closing this account to open another one in place");
    crate::leave_calls_for_account_close(&app, handle).await;
    crate::sync_link::clear_url();
    crate::account_log::stop();
    crate::key_handoff::stop();
    crate::community_sync::stop();
    let stopped = stop_tasks();
    let node = app.net.lock().await.take();
    if let Some(node) = node {
        if let Err(e) = node.shutdown().await {
            tracing::warn!("the old account's connection did not close cleanly: {e}");
        }
    }
    crate::forget_account_statics();
    app.forget_account();
    tracing::info!(stopped, "the account is closed; this copy of Commy can open another one");
    RELOAD_PENDING.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn reload_if_closed(handle: &AppHandle) {
    if RELOAD_PENDING.swap(false, std::sync::atomic::Ordering::SeqCst) {
        let _ = handle.emit("account-closed", ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_an_account_stops_its_tasks() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let task = spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                if tx.send(()).is_err() {
                    return;
                }
            }
        });
        rx.recv().await.expect("the task runs while the account is open");
        assert!(stop_tasks() >= 1);
        assert!(task.await.is_err(), "the task was stopped");
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(rx.try_recv().is_err(), "nothing runs after the account closed");
    }

    #[test]
    fn a_closed_account_folder_can_be_replaced() {
        let dir = crate::state::DataDir::default();
        assert!(dir.get().is_none());
        dir.set("first".into()).unwrap();
        assert!(dir.set("second".into()).is_err(), "an open folder is not replaced");
        assert_eq!(dir.get().unwrap().to_str(), Some("first"));
        dir.clear();
        assert!(dir.get().is_none());
        dir.set("second".into()).unwrap();
        assert_eq!(dir.get().unwrap().to_str(), Some("second"));
    }

    #[test]
    fn forgetting_an_account_empties_what_it_loaded() {
        let app = App::default();
        *app.profile.lock_or_recover() = Some(commy_core::UserProfile::new("ferris"));
        app.data_dir.set("account".into()).unwrap();
        app.awaiting_snapshot.lock_or_recover().insert(commy_core::ServerId::new());
        app.forget_account();
        assert!(app.profile.lock_or_recover().is_none());
        assert!(app.data_dir().is_none());
        assert!(app.awaiting_snapshot.lock_or_recover().is_empty());
    }
}
