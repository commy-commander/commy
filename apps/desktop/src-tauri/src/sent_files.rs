use commy_net::shelf::Shelf;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const SENT_FILE_GONE: &str = "your copy of this file is no longer on this device";

pub(crate) const NOT_THE_SENT_FILE: &str = "that is not the file you sent";

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

pub(crate) fn usual_folders(handle: &tauri::AppHandle) -> Vec<PathBuf> {
    use tauri::Manager;
    let paths = handle.path();
    let mut folders = vec![paths.download_dir(), paths.video_dir(), paths.picture_dir()];
    #[cfg(desktop)]
    folders.push(paths.desktop_dir());
    folders.push(paths.document_dir());
    folders.into_iter().filter_map(Result::ok).collect()
}

pub(crate) fn places_to_look(folders: &[PathBuf], name: &str) -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();
    for place in folders.iter().map(|folder| folder.join(name)) {
        if !places.contains(&place) {
            places.push(place);
        }
    }
    places
}

pub(crate) async fn find_again(shelf: &Shelf, blob: &str, size: u64, places: Vec<PathBuf>) -> bool {
    for place in places {
        let same_size = tokio::fs::metadata(&place).await.is_ok_and(|meta| meta.is_file() && meta.len() == size);
        if !same_size {
            continue;
        }
        match keep_again(shelf, &place, blob).await {
            Ok(true) => {
                tracing::info!("found a file sent from here in one of the usual folders and shared it again");
                return true;
            }
            Ok(false) => {}
            Err(e) => tracing::debug!("a file that could be one sent from here was not read: {e:#}"),
        }
    }
    false
}

pub(crate) async fn keep_again(shelf: &Shelf, path: &Path, blob: &str) -> anyhow::Result<bool> {
    shelf.keep_sent_file_if(path, blob, now_secs()).await
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
    fn the_usual_folders_are_each_looked_in_once() {
        let downloads = PathBuf::from("home").join("Downloads");
        let videos = PathBuf::from("home").join("Videos");
        let places = places_to_look(&[downloads.clone(), videos.clone(), downloads.clone()], "clip.mp4");
        assert_eq!(places, vec![downloads.join("clip.mp4"), videos.join("clip.mp4")]);
    }

    #[tokio::test]
    async fn a_file_sent_before_sent_files_were_kept_is_found_in_a_usual_folder() {
        let dir = std::env::temp_dir().join(format!("commy-sent-again-{}", rand::random::<u64>()));
        let downloads = dir.join("Downloads");
        let videos = dir.join("Videos");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&videos).unwrap();
        let video: Vec<u8> = (0..120_000u32).map(|i| (i % 227) as u8).collect();
        std::fs::write(downloads.join("clip.mp4"), vec![1u8; video.len()]).unwrap();
        std::fs::write(videos.join("clip.mp4"), &video).unwrap();
        let blob = commy_net::node::blob_hash(&video);
        let shelf = Shelf::open(&dir.join("shelf")).await.unwrap();
        let folders = [downloads.clone(), videos.clone()];
        assert!(!find_again(&shelf, &blob, video.len() as u64 + 1, places_to_look(&folders, "clip.mp4")).await, "a different size is never read");
        assert!(!find_again(&shelf, &blob, video.len() as u64, places_to_look(&folders, "other.mp4")).await);
        assert!(find_again(&shelf, &blob, video.len() as u64, places_to_look(&folders, "clip.mp4")).await, "the same name and size with a different hash is skipped");
        assert_eq!(shelf.bytes(&blob).await, Some(video));
        shelf.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sent_files_are_kept_for_thirty_days() {
        let day = 24 * 60 * 60;
        let now = 100 * day;
        assert_eq!(keep_from(now), 70 * day);
        assert_eq!(keep_from(5), 0, "a clock near the epoch drops nothing");
    }
}
