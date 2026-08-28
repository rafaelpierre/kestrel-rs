//! Optional persistent cache for extracted page text.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::search::{KestrelError, canonical_url};

/// A TTL-bound disk cache keyed by canonical URL and extraction limit.
#[derive(Clone, Debug)]
pub struct PageCache {
    directory: PathBuf,
    ttl: Duration,
    max_entries: usize,
}

impl PageCache {
    pub fn new(directory: impl Into<PathBuf>, ttl: Duration) -> Result<Self, KestrelError> {
        if ttl.is_zero() {
            return Err(KestrelError::InvalidRequest(
                "cache TTL must be greater than zero".into(),
            ));
        }
        Ok(Self {
            directory: directory.into(),
            ttl,
            max_entries: 1_000,
        })
    }

    pub fn with_max_entries(mut self, max_entries: usize) -> Result<Self, KestrelError> {
        if max_entries == 0 {
            return Err(KestrelError::InvalidRequest(
                "cache max entries must be at least 1".into(),
            ));
        }
        self.max_entries = max_entries;
        Ok(self)
    }

    /// Default per-user cache location used by the CLI.
    pub fn default_directory() -> Result<PathBuf, KestrelError> {
        home::home_dir()
            .map(|home| home.join(".cache").join("kestrel").join("pages"))
            .ok_or_else(|| KestrelError::InvalidRequest("home directory is unavailable".into()))
    }

    pub(crate) async fn get(&self, url: &str, content_limit: usize) -> Option<String> {
        let target = self.target(url, content_limit);
        let metadata = tokio::fs::metadata(&target).await.ok()?;
        let modified = metadata.modified().ok()?;
        let fresh = SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age <= self.ttl);
        if !fresh {
            let _ = tokio::fs::remove_file(target).await;
            return None;
        }
        tokio::fs::read_to_string(target).await.ok()
    }

    pub(crate) async fn put(
        &self,
        url: &str,
        content_limit: usize,
        content: &str,
    ) -> Result<(), KestrelError> {
        tokio::fs::create_dir_all(&self.directory).await?;
        let target = self.target(url, content_limit);
        let temporary = self
            .directory
            .join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        tokio::fs::write(&temporary, content).await?;
        if let Err(error) = tokio::fs::rename(&temporary, &target).await
            && tokio::fs::metadata(&target).await.is_err()
        {
            let _ = tokio::fs::remove_file(temporary).await;
            return Err(error.into());
        }
        Ok(())
    }

    pub(crate) async fn prune(&self) -> Result<(), KestrelError> {
        let mut directory = tokio::fs::read_dir(&self.directory).await?;
        let mut entries = Vec::new();
        while let Some(entry) = directory.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("txt") {
                continue;
            }
            let modified = entry
                .metadata()
                .await
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((modified, path));
        }
        if entries.len() > self.max_entries {
            entries.sort_unstable_by_key(|(modified, _)| *modified);
            let remove_count = entries.len() - self.max_entries;
            for (_, path) in entries.into_iter().take(remove_count) {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        Ok(())
    }

    fn target(&self, url: &str, content_limit: usize) -> PathBuf {
        let canonical = canonical_url(url);
        let key_url = if canonical.is_empty() {
            url
        } else {
            &canonical
        };
        let digest = Sha256::digest(format!("{key_url}\0{content_limit}").as_bytes());
        self.directory.join(format!("{digest:x}.txt"))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keys_by_canonical_url_and_content_limit() {
        let directory = tempfile::tempdir().unwrap();
        let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        cache
            .put("https://example.com/page?utm_source=test", 2_000, "cached")
            .await
            .unwrap();
        assert_eq!(
            cache
                .get("https://example.com/page", 2_000)
                .await
                .as_deref(),
            Some("cached")
        );
        assert_eq!(cache.get("https://example.com/page", 1_000).await, None);
    }
}
