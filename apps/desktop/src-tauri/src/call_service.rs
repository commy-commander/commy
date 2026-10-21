use std::sync::mpsc::Sender;
use std::sync::Mutex;

struct Calls {
    held: usize,
    keeper: Option<Sender<bool>>,
}

impl Calls {
    fn change(&mut self, by: isize) {
        self.held = self.held.saturating_add_signed(by);
        if let Some(keeper) = &self.keeper {
            let _ = keeper.send(self.held > 0);
        }
    }
}

static CALLS: Mutex<Calls> = Mutex::new(Calls { held: 0, keeper: None });

pub struct CallKeeper(());

impl CallKeeper {
    pub fn hold() -> Self {
        CALLS.lock().unwrap().change(1);
        CallKeeper(())
    }
}

impl Drop for CallKeeper {
    fn drop(&mut self) {
        CALLS.lock().unwrap().change(-1);
    }
}

#[cfg(any(target_os = "android", test))]
fn follow(changes: std::sync::mpsc::Receiver<bool>, mut apply: impl FnMut(bool) -> Result<(), String>) {
    let mut applied = false;
    while let Ok(mut wanted) = changes.recv() {
        while let Ok(next) = changes.try_recv() {
            wanted = next;
        }
        if wanted == applied {
            continue;
        }
        match apply(wanted) {
            Ok(()) => applied = wanted,
            Err(e) => tracing::warn!("keeping the call running in the background: {e}"),
        }
    }
}

#[cfg(target_os = "android")]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("call-service")
        .setup(|_app, api| {
            let plugin = api.register_android_plugin("chat.commy.mobile", "CallServicePlugin")?;
            let (tx, rx) = std::sync::mpsc::channel();
            {
                let mut calls = CALLS.lock().unwrap();
                let _ = tx.send(calls.held > 0);
                calls.keeper = Some(tx);
            }
            std::thread::Builder::new().name("call-service".into()).spawn(move || {
                follow(rx, |in_call| {
                    let command = if in_call { "start" } else { "stop" };
                    plugin
                        .run_mobile_plugin::<serde_json::Value>(command, serde_json::json!({}))
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
            })?;
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_changes_settles_on_the_last_one() {
        let (tx, rx) = std::sync::mpsc::channel();
        for wanted in [true, false, true, true] {
            tx.send(wanted).unwrap();
        }
        drop(tx);
        let mut applied = Vec::new();
        follow(rx, |wanted| {
            applied.push(wanted);
            Ok(())
        });
        assert_eq!(applied, vec![true]);
    }

    #[test]
    fn a_refused_start_is_tried_again_on_the_next_change() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(true).unwrap();
        let mut tx = Some(tx);
        let mut tries = Vec::new();
        follow(rx, |wanted| {
            tries.push(wanted);
            match tries.len() {
                1 => {
                    tx.as_ref().unwrap().send(true).unwrap();
                    Err("not now".into())
                }
                2 => {
                    tx.take().unwrap().send(false).unwrap();
                    Ok(())
                }
                _ => Ok(()),
            }
        });
        assert_eq!(tries, vec![true, true, false]);
    }

    #[test]
    fn a_new_call_replacing_the_old_one_keeps_the_service_up() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut calls = Calls { held: 0, keeper: Some(tx) };
        calls.change(1);
        calls.change(1);
        calls.change(-1);
        calls.change(-1);
        calls.change(-1);
        let seen: Vec<bool> = rx.try_iter().collect();
        assert_eq!(seen, vec![true, true, true, false, false]);
    }
}
