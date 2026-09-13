//! Optional persistent cache for extracted page text.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::search::KestrelError;

// Bump when extraction semantics or the persistent identity contract changes.
const EXTRACTION_CACHE_VERSION: &str = "page-text-v2";

/// A TTL-bound disk cache keyed by conservative request URL, extraction version and limit.
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
        // Search deduplication is a heuristic, not proof of HTTP resource identity.
        // Fragments are not sent to the server. Preserve query order, tracking
        // parameters, encoded paths and trailing slashes. URL parsing applies only
        // standard URL normalization (for example default ports and host case).
        let key_url = match url::Url::parse(url) {
            Ok(mut parsed) => {
                parsed.set_fragment(None);
                parsed.to_string()
            }
            Err(_) => url.to_owned(),
        };
        let digest = Sha256::digest(
            format!("{EXTRACTION_CACHE_VERSION}\0{key_url}\0{content_limit}").as_bytes(),
        );
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
    async fn keys_preserve_resource_distinctions_and_extraction_limit() {
        let _telemetry = crate::telemetry::test_export_guard();
        let directory = tempfile::tempdir().unwrap();
        let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        cache
            .put("https://example.com/page#one", 2_000, "cached")
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
    #[test]
    fn conservative_identity_and_legacy_invalidation() {
        let cache = PageCache::new("cache", Duration::from_secs(60)).unwrap();
        let base = "https://example.com/page";
        for distinct in [
            "https://example.com/page/",
            "https://example.com/page?utm_source=test",
            "https://example.com/page?a=1",
            "http://example.com/page",
        ] {
            assert_ne!(cache.target(base, 2000), cache.target(distinct, 2000));
        }
        for (left, right) in [
            ("https://example.com/a%2Fb", "https://example.com/a/b"),
            ("https://example.com/%70age", base),
            (
                "https://example.com/page?a=1&b=2",
                "https://example.com/page?b=2&a=1",
            ),
            (
                "https://example.com/page?a=1&a=2",
                "https://example.com/page?a=2&a=1",
            ),
        ] {
            assert_ne!(cache.target(left, 2000), cache.target(right, 2000));
        }
        for equivalent in [
            "https://EXAMPLE.com:443/page",
            "https://example.com/page#section",
        ] {
            assert_eq!(cache.target(base, 2000), cache.target(equivalent, 2000));
        }
        let legacy = Sha256::digest(format!("{base}\0{}", 2000).as_bytes());
        assert_ne!(
            cache.target(base, 2000),
            cache.directory.join(format!("{legacy:x}.txt"))
        );
    }
}
