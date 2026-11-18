use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_sync::pass::{ask, collect, current_key, IssueReply, PASS_ISSUE_PATH, PASS_KEYS_PATH};
use commy_sync::{carry_everywhere, Keeper, Wallet};

use super::Session;

pub(crate) const PASSES_KEY: &str = "commy.passes";
const SOONEST_MS: f64 = 60_000.0;
const LATEST_MS: f64 = 20.0 * 60_000.0;
const AFTER_FAILURE: Duration = Duration::from_secs(15 * 60);
const SAVE_EVERY: Duration = Duration::from_secs(5 * 60);

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(crate) fn spread_wait(unit: f64) -> Duration {
    Duration::from_millis((SOONEST_MS + unit.clamp(0.0, 1.0) * (LATEST_MS - SOONEST_MS)) as u64)
}

impl Session {
    pub(super) async fn tend_passes(self: Rc<Self>) {
        let saved = self.host.load(PASSES_KEY).and_then(|text| serde_json::from_str(&text).ok());
        let keeper = Keeper::new(Wallet::resume(saved, &self.origin), now_ms);
        carry_everywhere(Some(keeper.purse()));
        loop {
            let low = n0_future::time::timeout(SAVE_EVERY, keeper.until_low()).await.is_ok();
            if self.sync.borrow().is_none() {
                carry_everywhere(None);
                return;
            }
            self.keep_passes(&keeper);
            if !low {
                continue;
            }
            n0_future::time::sleep(spread_wait(js_sys::Math::random())).await;
            if self.refill_passes(&keeper).await.is_err() {
                n0_future::time::sleep(AFTER_FAILURE).await;
            }
            self.keep_passes(&keeper);
        }
    }

    fn keep_passes(&self, keeper: &Keeper) {
        if let Ok(text) = serde_json::to_string(&keeper.wallet()) {
            self.host.save(PASSES_KEY, &text);
        }
    }

    async fn refill_passes(&self, keeper: &Keeper) -> Result<usize, String> {
        let wanted = keeper.wanted();
        if wanted == 0 {
            return Ok(0);
        }
        let keys = self.request("GET", PASS_KEYS_PATH, None).await?;
        if keys.status == 404 {
            n0_future::time::sleep(AFTER_FAILURE * 4).await;
            return Ok(0);
        }
        if !keys.ok() {
            return Err(format!("the account server answered {}", keys.status));
        }
        let key = current_key(&keys.body, now_ms()).map_err(|e| e.to_string())?;
        let (request, pending) = ask(&key, wanted, &mut rand::thread_rng()).map_err(|e| e.to_string())?;
        let body = serde_json::to_value(&request).map_err(|e| e.to_string())?;
        let reply = self.request("POST", PASS_ISSUE_PATH, Some(body)).await?;
        if !reply.ok() {
            return Err(format!("the account server answered {}", reply.status));
        }
        let answer: IssueReply = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
        let passes = collect(&key, pending, &answer);
        let got = passes.len();
        keeper.add(passes);
        Ok(got)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refills_wait_a_while_so_they_do_not_line_up_with_spending() {
        assert_eq!(spread_wait(0.0), Duration::from_secs(60));
        assert_eq!(spread_wait(1.0), Duration::from_secs(20 * 60));
        assert_eq!(spread_wait(7.0), Duration::from_secs(20 * 60));
        assert!(spread_wait(0.5) > Duration::from_secs(60));
    }
}
