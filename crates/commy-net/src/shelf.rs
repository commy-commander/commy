use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use iroh_blobs::api::blobs::{AddPathOptions, ExportMode, ExportOptions, ImportMode};
use iroh_blobs::store::fs::options::Options;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::store::GcConfig;
use iroh_blobs::api::remote::GetProgressItem;
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash, HashAndFormat};
use futures_lite::StreamExt;

pub use crate::node::FILES_ALPN;

const SENT_TAG_PREFIX: &str = "sent/";

const OPENED_TAG_PREFIX: &str = "opened/";

const STORED_TAG_PREFIX: &str = "stored/";

const KEPT_TAG_PREFIXES: [&str; 3] = [SENT_TAG_PREFIX, OPENED_TAG_PREFIX, STORED_TAG_PREFIX];

const PART_SUFFIX: &str = ".part";
const FORGET_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct Shelf {
    store: FsStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shared {
    pub size: u64,
}

impl Shelf {
    pub async fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).context("could not create the shared files folder")?;
        let mut options = Options::new(dir);
        options.gc = Some(GcConfig { interval: FORGET_AFTER, add_protected: None });
        let store = FsStore::load_with_opts(dir.join("blobs.db"), options).await.context("could not open the shared files list")?;
        Ok(Self { store })
    }

    pub fn protocol(&self) -> BlobsProtocol {
        BlobsProtocol::new(&self.store, None)
    }

    pub async fn share(&self, path: &Path) -> Result<(String, u64)> {
        let path = std::path::absolute(path).context("could not find that file")?;
        let options = AddPathOptions { path, format: BlobFormat::Raw, mode: ImportMode::TryReference };
        let hash = self.store.add_path_with_opts(options).with_tag().await.context("could not read that file")?.hash;
        let size = self.shared(&hash.to_hex()).await.map(|s| s.size).unwrap_or_default();
        Ok((hash.to_hex().to_string(), size))
    }

    pub async fn keep_sent_bytes(&self, bytes: Vec<u8>, at_secs: u64) -> Result<String> {
        let held = self.store.add_bytes(bytes).temp_tag().await.context("could not keep the sent file")?;
        self.mark_sent(held.hash(), at_secs).await
    }

    pub async fn keep_sent_file(&self, path: &Path, at_secs: u64) -> Result<String> {
        let path = std::path::absolute(path).context("could not find that file")?;
        let options = AddPathOptions { path, format: BlobFormat::Raw, mode: ImportMode::TryReference };
        let held = self.store.add_path_with_opts(options).temp_tag().await.context("could not keep the sent file")?;
        self.mark_sent(held.hash(), at_secs).await
    }

    async fn mark_sent(&self, hash: Hash, at_secs: u64) -> Result<String> {
        self.mark(SENT_TAG_PREFIX, hash, at_secs).await
    }

    pub async fn keep_opened_bytes(&self, bytes: Vec<u8>, at_secs: u64) -> Result<String> {
        let held = self.store.add_bytes(bytes).temp_tag().await.context("could not keep the opened file")?;
        self.mark(OPENED_TAG_PREFIX, held.hash(), at_secs).await
    }

    pub async fn keep_opened_file(&self, path: &Path, at_secs: u64) -> Result<String> {
        let path = std::path::absolute(path).context("could not find that file")?;
        let options = AddPathOptions { path, format: BlobFormat::Raw, mode: ImportMode::TryReference };
        let held = self.store.add_path_with_opts(options).temp_tag().await.context("could not keep the downloaded file")?;
        self.mark(OPENED_TAG_PREFIX, held.hash(), at_secs).await
    }

    pub async fn store_for_others(&self, connection: iroh::endpoint::Connection, hash: &str, max: u64, at_secs: u64) -> Result<u64> {
        let hash: Hash = hash.parse().context("invalid file hash")?;
        let _held = self.store.tags().temp_tag(HashAndFormat::raw(hash)).await.context("could not keep the file")?;
        let mut progress = std::pin::pin!(self.store.remote().fetch(connection, HashAndFormat::raw(hash)).stream());
        while let Some(item) = progress.next().await {
            match item {
                GetProgressItem::Progress(read) => crate::node::check_blob_size(read, max)?,
                GetProgressItem::Done(_) => break,
                GetProgressItem::Error(e) => return Err(anyhow::Error::new(e).context("file download failed")),
            }
        }
        let shared = self.shared(&hash.to_hex()).await.context("file download failed: incomplete")?;
        crate::node::check_blob_size(shared.size, max)?;
        self.mark(STORED_TAG_PREFIX, hash, at_secs).await?;
        Ok(shared.size)
    }

    pub async fn stored_for_others_bytes(&self) -> Result<u64> {
        let tags = self.store.tags().list_prefix(STORED_TAG_PREFIX).await.context("could not read the shared files list")?;
        let hashes: std::collections::BTreeSet<Hash> = tags.filter_map(|tag| tag.ok().map(|t| t.hash)).collect().await;
        let mut total = 0;
        for hash in hashes {
            total += self.shared(&hash.to_hex()).await.map_or(0, |s| s.size);
        }
        Ok(total)
    }

    async fn mark(&self, prefix: &str, hash: Hash, at_secs: u64) -> Result<String> {
        self.store.tags().set(kept_tag(prefix, at_secs, &hash), hash).await.context("could not keep the file")?;
        Ok(hash.to_hex().to_string())
    }

    pub async fn drop_kept_before(&self, cutoff_secs: u64) -> Result<u64> {
        let mut gone = 0;
        for prefix in KEPT_TAG_PREFIXES {
            let tags = self.store.tags().list_prefix(prefix).await.context("could not read the shared files list")?;
            let stale: Vec<_> = tags
                .filter_map(|tag| tag.ok().map(|t| t.name).filter(|name| kept_at(prefix, name.as_ref()).is_some_and(|at| at < cutoff_secs)))
                .collect()
                .await;
            for name in stale {
                gone += self.store.tags().delete(name).await.context("could not drop an old kept file")?;
            }
        }
        Ok(gone)
    }

    pub async fn trim_to(&self, max_bytes: u64) -> Result<u64> {
        let mut kept = Vec::new();
        for prefix in KEPT_TAG_PREFIXES {
            let tags = self.store.tags().list_prefix(prefix).await.context("could not read the shared files list")?;
            let found: Vec<_> = tags
                .filter_map(|tag| tag.ok().and_then(|t| kept_at(prefix, t.name.as_ref()).map(|at| (at, t.name, t.hash))))
                .collect()
                .await;
            kept.extend(found);
        }
        kept.sort_by(|a, b| b.0.cmp(&a.0));
        let mut fits = std::collections::HashSet::new();
        let mut total = 0u64;
        let mut full = false;
        for (_, _, hash) in &kept {
            if full || fits.contains(hash) {
                continue;
            }
            let size = self.shared(&hash.to_hex()).await.map_or(0, |s| s.size);
            match total.checked_add(size).filter(|sum| *sum <= max_bytes) {
                Some(sum) => {
                    total = sum;
                    fits.insert(*hash);
                }
                None => full = true,
            }
        }
        let mut gone = 0;
        for (_, name, hash) in kept {
            if !fits.contains(&hash) {
                gone += self.store.tags().delete(name).await.context("could not drop an old kept file")?;
            }
        }
        Ok(gone)
    }

    pub async fn bytes(&self, hash: &str) -> Option<Vec<u8>> {
        let hash: Hash = hash.parse().ok()?;
        self.store.get_bytes(hash).await.ok().map(|bytes| bytes.to_vec())
    }

    pub async fn shared(&self, hash: &str) -> Option<Shared> {
        let hash: Hash = hash.parse().ok()?;
        match self.store.status(hash).await.ok()? {
            iroh_blobs::api::blobs::BlobStatus::Complete { size } => Some(Shared { size }),
            _ => None,
        }
    }

    pub async fn forget(&self, hash: &str) -> Result<u64> {
        let hash: Hash = hash.parse().context("invalid file hash")?;
        let tags = self.store.tags().list().await.context("could not read the shared files list")?;
        let names: Vec<_> = tags.filter_map(|tag| tag.ok().filter(|t| t.hash == hash).map(|t| t.name)).collect().await;
        let mut gone = 0;
        for name in names {
            gone += self.store.tags().delete(name).await.context("could not stop sharing that file")?;
        }
        Ok(gone)
    }

    pub async fn copy_out(&self, hash: &str, target: &Path) -> Result<u64> {
        let hash: Hash = hash.parse().context("invalid file hash")?;
        let target = std::path::absolute(target).context("could not use that folder")?;
        let options = ExportOptions { hash, mode: ExportMode::Copy, target };
        self.store.export_with_opts(options).finish().await.context("could not copy the file")
    }

    pub async fn shutdown(&self) {
        let _ = self.store.shutdown().await;
    }
}

fn kept_tag(prefix: &str, at_secs: u64, hash: &Hash) -> String {
    format!("{prefix}{at_secs:020}/{}", hash.to_hex())
}

fn kept_at(prefix: &str, name: &[u8]) -> Option<u64> {
    let name = std::str::from_utf8(name).ok()?.strip_prefix(prefix)?;
    name.split('/').next()?.parse().ok()
}

pub fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(PART_SUFFIX);
    target.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_land_next_to_the_target() {
        let target = Path::new("/home/sam/Downloads/movie.mkv");
        assert_eq!(part_path(target), Path::new("/home/sam/Downloads/movie.mkv.part"));
    }

    #[tokio::test]
    async fn a_shared_file_is_hashed_where_it_is_and_kept_after_a_restart() {
        let dir = tempdir();
        let file = dir.join("big.bin");
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file, &bytes).unwrap();
        let shelf = Shelf::open(&dir.join("shelf")).await.unwrap();
        let (hash, size) = shelf.share(&file).await.unwrap();
        assert_eq!(hash, crate::node::blob_hash(&bytes));
        assert_eq!(size, bytes.len() as u64);
        let data_dir = dir.join("shelf").join("data");
        let stored: u64 = std::fs::read_dir(&data_dir)
            .map(|entries| entries.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
            .unwrap_or(0);
        assert!(stored < bytes.len() as u64 / 4, "the file is referenced, not copied ({stored} bytes stored)");
        shelf.shutdown().await;
        let reopened = Shelf::open(&dir.join("shelf")).await.unwrap();
        assert_eq!(reopened.shared(&hash).await, Some(Shared { size: bytes.len() as u64 }));
        let copy = dir.join("copy.bin");
        assert_eq!(reopened.copy_out(&hash, &copy).await.unwrap(), bytes.len() as u64);
        assert_eq!(std::fs::read(&copy).unwrap(), bytes);
        assert_eq!(reopened.shared(&crate::node::blob_hash(b"other")).await, None);
        assert_eq!(reopened.forget(&hash).await.unwrap(), 1);
        assert_eq!(reopened.forget(&hash).await.unwrap(), 0);
        assert!(file.exists(), "the original file stays where it was");
        reopened.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_sent_file_is_kept_across_a_restart_until_it_is_old_enough_to_drop() {
        let dir = tempdir();
        let picked = dir.join("clip.mp4");
        let pasted: Vec<u8> = (0..150_000u32).map(|i| (i % 239) as u8).collect();
        let video: Vec<u8> = (0..180_000u32).map(|i| (i % 233) as u8).collect();
        std::fs::write(&picked, &video).unwrap();
        let shelf = Shelf::open(&dir.join("shelf")).await.unwrap();
        let old = shelf.keep_sent_bytes(pasted.clone(), 1_000).await.unwrap();
        let fresh = shelf.keep_sent_file(&picked, 5_000).await.unwrap();
        assert_eq!(old, crate::node::blob_hash(&pasted));
        assert_eq!(fresh, crate::node::blob_hash(&video));
        shelf.shutdown().await;
        let reopened = Shelf::open(&dir.join("shelf")).await.unwrap();
        assert_eq!(reopened.bytes(&old).await, Some(pasted));
        assert_eq!(reopened.bytes(&fresh).await, Some(video.clone()));
        assert_eq!(reopened.drop_kept_before(2_000).await.unwrap(), 1);
        assert_eq!(reopened.drop_kept_before(2_000).await.unwrap(), 0);
        assert!(reopened.shared(&fresh).await.is_some(), "newer sends stay");
        assert_eq!(reopened.drop_kept_before(9_000).await.unwrap(), 1);
        assert!(picked.exists(), "dropping a sent file never touches the original");
        reopened.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_full_cache_lets_go_of_the_oldest_files_first() {
        let dir = tempdir();
        let shelf = Shelf::open(&dir.join("shelf")).await.unwrap();
        let file = |seed: u32| -> Vec<u8> { (0..100_000u32).map(|i| ((i + seed) % 241) as u8).collect() };
        let oldest = shelf.keep_sent_bytes(file(1), 1_000).await.unwrap();
        let older = shelf.keep_opened_bytes(file(2), 2_000).await.unwrap();
        let newer = shelf.keep_opened_bytes(file(3), 3_000).await.unwrap();
        let opened_again = shelf.keep_opened_bytes(file(1), 4_000).await.unwrap();
        assert_eq!(opened_again, oldest);
        assert_eq!(shelf.trim_to(250_000).await.unwrap(), 1, "only the file whose newest use is oldest goes");
        let tags: Vec<_> = shelf.store.tags().list().await.unwrap().filter_map(|t| t.ok().map(|t| t.hash.to_hex().to_string())).collect().await;
        assert!(tags.contains(&oldest) && tags.contains(&newer));
        assert!(!tags.contains(&older));
        assert_eq!(shelf.trim_to(250_000).await.unwrap(), 0);
        assert_eq!(shelf.trim_to(0).await.unwrap(), 3);
        shelf.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sent_tags_sort_by_when_they_were_sent() {
        let hash = Hash::new(b"x");
        assert!(kept_tag(SENT_TAG_PREFIX, 999, &hash) < kept_tag(SENT_TAG_PREFIX, 1_000, &hash));
        assert_eq!(kept_at(SENT_TAG_PREFIX, kept_tag(SENT_TAG_PREFIX, 1_000, &hash).as_bytes()), Some(1_000));
        assert_eq!(kept_at(SENT_TAG_PREFIX, b"auto-123"), None);
        assert_eq!(kept_at(OPENED_TAG_PREFIX, kept_tag(SENT_TAG_PREFIX, 1_000, &hash).as_bytes()), None);
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("commy-shelf-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
