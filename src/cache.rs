//! Optional persistent cache for extracted page text.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(crate) const CACHE_IO_CONCURRENCY: usize = 4;
const PRUNE_SCAN_LIMIT: usize = 4_096;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::search::{KestrelError, canonical_url};

/// A TTL-bound disk cache keyed by canonical URL and extraction limit.
#[derive(Clone, Debug)]
pub struct PageCache {
    directory: PathBuf,
    ttl: Duration,
    max_entries: usize,
    io: Arc<Semaphore>,
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
            io: Arc::new(Semaphore::new(CACHE_IO_CONCURRENCY)),
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
        let ttl = self.ttl;
        self.run_io(move || {
            let metadata = match std::fs::metadata(&target) {
                Ok(metadata) => metadata,
                Err(_) => return Ok(None),
            };
            let fresh = metadata.modified().ok().is_some_and(|modified| {
                SystemTime::now()
                    .duration_since(modified)
                    .is_ok_and(|age| age <= ttl)
            });
            if !fresh {
                // Expiry is a miss; cleanup must not delete a concurrent writer's replacement.
                return Ok(None);
            }
            let max_bytes = content_limit.saturating_mul(4);
            if metadata.len() > max_bytes as u64 {
                return Ok(None);
            }
            let file = match std::fs::File::open(target) {
                Ok(file) => file,
                Err(_) => return Ok(None),
            };
            let mut text = String::new();
            if file
                .take((max_bytes as u64).saturating_add(1))
                .read_to_string(&mut text)
                .is_err()
                || text.len() > max_bytes
                || text.chars().count() > content_limit
            {
                return Ok(None);
            }
            Ok(Some(text))
        })
        .await
        .ok()
        .flatten()
    }

    // Admission happens before spawning, and the worker owns the permit. Dropping
    // an async caller cannot admit unlimited replacement work behind a blocked disk.
    async fn run_io<T: Send + 'static>(
        &self,
        action: impl FnOnce() -> Result<T, std::io::Error> + Send + 'static,
    ) -> Result<T, KestrelError> {
        let permit = self
            .io
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            action()
        })
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .map_err(Into::into)
    }

    pub(crate) async fn put(
        &self,
        url: &str,
        content_limit: usize,
        content: &str,
    ) -> Result<(), KestrelError> {
        let cache = self.clone();
        let target = self.target(url, content_limit);
        let content = content.to_owned();
        self.run_io(move || {
            std::fs::create_dir_all(&cache.directory)?;
            let temporary = cache
                .directory
                .join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
            if let Err(error) = std::fs::write(&temporary, content)
                .and_then(|()| std::fs::rename(&temporary, &target))
            {
                let _ = std::fs::remove_file(temporary);
                return Err(error);
            }
            Ok(())
        })
        .await
    }

    pub(crate) async fn prune(&self) -> Result<(), KestrelError> {
        let cache = self.clone();
        self.run_io(move || {
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&cache.directory)?.take(PRUNE_SCAN_LIMIT) {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("txt") {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                entries.push((modified, path));
            }
            if entries.len() > cache.max_entries {
                entries.sort_unstable_by_key(|(modified, _)| *modified);
                let remove_count = entries.len() - cache.max_entries;
                for (_, path) in entries.into_iter().take(remove_count) {
                    let _ = std::fs::remove_file(path);
                }
            }
            Ok(())
        })
        .await
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
        let _telemetry = crate::telemetry::test_export_guard();
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_cache_reads_keep_blocking_work_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        cache
            .put("https://example.test/page", 2000, "warm text")
            .await
            .unwrap();
        let client = crate::KestrelClient::new().unwrap();
        let mut releases = Vec::new();
        let mut workers = Vec::new();
        for _ in 0..CACHE_IO_CONCURRENCY {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel::<()>();
            releases.push(release);
            let cache = cache.clone();
            workers.push(tokio::spawn(async move {
                cache
                    .run_io(move || {
                        let _ = started.send(());
                        let _ = wait.recv();
                        Ok(())
                    })
                    .await
                    .unwrap();
            }));
            ready.await.unwrap();
        }
        for _ in 0..2 {
            let started = std::time::Instant::now();
            let report = client
                .fetch_all_cached_detailed(
                    &["https://example.test/page".into()],
                    &crate::FetchOptions::default(),
                    &cache,
                    Some(Duration::from_millis(50)),
                )
                .await
                .unwrap();
            assert!(report.budget_exhausted);
            assert_eq!(report.cache_hits, 0);
            assert_eq!(report.cancelled, 1);
            assert_eq!(report.contents, vec![None]);
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(
                cache.io.available_permits(),
                0,
                "cancelled readers cannot free active workers' permits"
            );
        }
        drop(releases);
        for worker in workers {
            worker.await.unwrap();
        }
        assert_eq!(cache.io.available_permits(), CACHE_IO_CONCURRENCY);
        assert_eq!(
            cache
                .get("https://example.test/page", 2000)
                .await
                .as_deref(),
            Some("warm text")
        );
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_deadline_preserves_warm_and_new_page_results() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let warm = format!("{base}/warm");
        let cold = format!("{base}/cold");
        cache.put(&warm, 2000, "warm evidence").await.unwrap();
        let (requested, request_ready) = tokio::sync::oneshot::channel();
        let (release_response, response_ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 16384);
            }
            assert!(request.starts_with(b"GET /cold "));
            requested.send(()).unwrap();
            response_ready.await.unwrap();
            let body = "new evidence";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let client = crate::KestrelClient::new().unwrap();
        let operation_cache = cache.clone();
        let urls = [warm, cold.clone()];
        let operation = tokio::spawn(async move {
            client
                .fetch_all_cached_detailed(
                    &urls,
                    &crate::FetchOptions::default(),
                    &operation_cache,
                    Some(Duration::from_millis(500)),
                )
                .await
                .unwrap()
        });
        request_ready.await.unwrap();
        // Cache reads finished before the request. Block subsequent persistence,
        // then release actual HTTP bytes; no timing sleep decides the boundary.
        let blocked = cache
            .io
            .clone()
            .acquire_many_owned(CACHE_IO_CONCURRENCY as u32)
            .await
            .unwrap();
        release_response.send(()).unwrap();
        let report = tokio::time::timeout(Duration::from_secs(3), operation)
            .await
            .unwrap()
            .unwrap();
        assert!(report.budget_exhausted);
        assert_eq!(report.cache_hits, 1);
        assert_eq!(report.cancelled, 0);
        assert_eq!(
            report.contents,
            vec![Some("warm evidence".into()), Some("new evidence".into())]
        );
        drop(blocked);
        server.await.unwrap();
        assert_eq!(cache.get(&cold, 2000).await, None);
    }
    #[tokio::test]
    async fn prune_wait_is_cancellable_and_directory_work_is_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let cache = PageCache::new(directory.path(), Duration::from_secs(60))
            .unwrap()
            .with_max_entries(1)
            .unwrap();
        for index in 0..5000 {
            std::fs::write(directory.path().join(format!("{index}.txt")), "x").unwrap();
        }
        let blocked = cache
            .io
            .clone()
            .acquire_many_owned(CACHE_IO_CONCURRENCY as u32)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), cache.prune())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 5000);
        drop(blocked);
        cache.prune().await.unwrap();
        let remaining = std::fs::read_dir(directory.path()).unwrap().count();
        assert!(
            remaining > 1 && remaining < 5000,
            "one maintenance pass must make progress without scanning the entire oversized directory"
        );
    }
}
