use std::path::{Path, PathBuf};
use std::time::Duration;

use commy_sync::pass::{ask, collect, current_key, IssueReply, PASS_ISSUE_PATH, PASS_KEYS_PATH};
use commy_sync::{carry_everywhere, Keeper, Wallet};
use rand::Rng;
use tauri::{AppHandle, Manager};

use crate::entry_list::{self, Session};
use crate::state::App;

const FILE: &str = "new-server-passes.json";
const SOONEST: Duration = Duration::from_secs(60);
const LATEST: Duration = Duration::from_secs(20 * 60);
const AFTER_FAILURE: Duration = Duration::from_secs(15 * 60);
const SAVE_EVERY: Duration = Duration::from_secs(5 * 60);

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub(crate) fn spread_wait(rng: &mut impl Rng) -> Duration {
    Duration::from_millis(rng.gen_range(SOONEST.as_millis() as u64..=LATEST.as_millis() as u64))
}

pub async fn run(handle: AppHandle) {
    let app = handle.state::<App>();
    let (Some(dir), Some(session)) = (app.data_dir().cloned(), entry_list::session(&app)) else { return };
    let keeper = Keeper::new(Wallet::resume(crate::persist::read_json(&path_in(&dir)), &session.server), now_ms);
    carry_everywhere(Some(keeper.purse()));
    let save = |keeper: &Keeper| {
        if let Err(e) = crate::persist::write_json(&path_in(&dir), &keeper.wallet()) {
            tracing::debug!("new server passes not saved: {e}");
        }
    };
    loop {
        if tokio::time::timeout(SAVE_EVERY, keeper.until_low()).await.is_err() {
            save(&keeper);
            continue;
        }
        save(&keeper);
        let wait = spread_wait(&mut rand::thread_rng());
        tokio::time::sleep(wait).await;
        let Some(session) = entry_list::session(&app) else { continue };
        match refill(&app, &session, &keeper).await {
            Ok(got) => tracing::debug!("new server passes: {got} more"),
            Err(e) => {
                tracing::debug!("new server passes: {e}");
                tokio::time::sleep(AFTER_FAILURE).await;
            }
        }
        save(&keeper);
    }
}

async fn refill(app: &App, session: &Session, keeper: &Keeper) -> Result<usize, String> {
    let wanted = keeper.wanted();
    if wanted == 0 {
        return Ok(0);
    }
    let base = session.server.trim_end_matches('/');
    let keys = app.http.get(format!("{base}{PASS_KEYS_PATH}")).send().await.map_err(|e| e.to_string())?;
    if keys.status() == reqwest::StatusCode::NOT_FOUND {
        tokio::time::sleep(LATEST * 3).await;
        return Ok(0);
    }
    let key = current_key(&keys.text().await.map_err(|e| e.to_string())?, now_ms()).map_err(|e| e.to_string())?;
    let (request, pending) = ask(&key, wanted, &mut rand::thread_rng()).map_err(|e| e.to_string())?;
    let res = app.http.post(format!("{base}{PASS_ISSUE_PATH}")).bearer_auth(&session.token).json(&request).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("the account server answered {}", res.status()));
    }
    let reply: IssueReply = res.json().await.map_err(|e| e.to_string())?;
    let passes = collect(&key, pending, &reply);
    let got = passes.len();
    keeper.add(passes);
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refills_wait_a_while_so_they_do_not_line_up_with_spending() {
        let mut rng = rand::thread_rng();
        for _ in 0..200 {
            let wait = spread_wait(&mut rng);
            assert!((SOONEST..=LATEST).contains(&wait));
        }
    }
}
