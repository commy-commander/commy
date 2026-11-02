use commy_net::shelf::Shelf;
use std::path::Path;
use std::time::Duration;

const KEEP_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);

const SWEEP_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

pub(crate) async fn keep(shelf: &Shelf, bytes: Option<Vec<u8>>, picked_from: Option<&Path>) {
    let now = now_secs();
    let kept = match (picked_from, bytes) {
        (Some(path), _) => shelf.keep_sent_file(path, now).await,
        (None, Some(bytes)) => shelf.keep_sent_bytes(bytes, now).await,
        (None, None) => return,
    };
    if let Err(e) = kept {
        tracing::warn!("a sent file stays available only until Commy closes: {e:#}");
    }
}

pub(crate) async fn sweep(shelf: Shelf) {
    loop {
        match shelf.drop_sent_before(keep_from(now_secs())).await {
            Ok(0) => {}
            Ok(dropped) => tracing::info!("stopped keeping {dropped} files sent over 30 days ago"),
            Err(e) => {
                tracing::debug!("sent files are no longer swept: {e:#}");
                return;
            }
        }
        tokio::time::sleep(SWEEP_EVERY).await;
    }
}

fn keep_from(now_secs: u64) -> u64 {
    now_secs.saturating_sub(KEEP_FOR.as_secs())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_files_are_kept_for_thirty_days() {
        let day = 24 * 60 * 60;
        let now = 100 * day;
        assert_eq!(keep_from(now), 70 * day);
        assert_eq!(keep_from(5), 0, "a clock near the epoch drops nothing");
    }
}
