use crate::lock_or_recover::LockOrRecover;
use std::sync::Mutex;
use std::time::Duration;

use commy_core::official_server::{self, ServerProof, Verdict};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::App;

const PROOF_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PROOF_BYTES: usize = 4 * 1024;

static LAST: Mutex<Option<(String, Verdict)>> = Mutex::new(None);

pub fn verdict_for(server: &str) -> Verdict {
    match LAST.lock_or_recover().as_ref() {
        Some((checked, verdict)) if checked == server => *verdict,
        _ => Verdict::Unproven,
    }
}

pub fn is_verified(server: &str) -> bool {
    worth_checking(server) && verdict_for(server) == Verdict::Official
}

pub fn is_impostor(server: &str) -> bool {
    worth_checking(server) && verdict_for(server) == Verdict::Impostor
}

fn worth_checking(server: &str) -> bool {
    official_server::is_official_origin(server) && !crate::account::is_bench_server(server)
}

async fn fetch_proof(app: &App, server: &str, nonce: &str) -> Option<ServerProof> {
    let res = app
        .http
        .get(format!("{server}{}", official_server::PROOF_PATH))
        .query(&[("nonce", nonce)])
        .timeout(PROOF_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() || res.content_length().is_some_and(|n| n as usize > MAX_PROOF_BYTES) {
        return None;
    }
    let body = res.bytes().await.ok()?;
    if body.len() > MAX_PROOF_BYTES {
        return None;
    }
    serde_json::from_slice(&body).ok()
}

pub async fn check(handle: &AppHandle, server: &str) -> Verdict {
    if !worth_checking(server) {
        return Verdict::Unproven;
    }
    let nonce = official_server::new_nonce();
    let proof = fetch_proof(&handle.state::<App>(), server, &nonce).await;
    let verdict = official_server::judge(server, &nonce, proof.as_ref());
    if verdict == Verdict::Impostor {
        tracing::warn!("the account server did not prove it holds the official key");
    }
    let changed = LAST.lock_or_recover().replace((server.to_string(), verdict)) != Some((server.to_string(), verdict));
    if changed {
        let _ = handle.emit("account-changed", ());
    }
    verdict
}
