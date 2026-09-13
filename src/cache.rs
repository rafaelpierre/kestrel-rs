//! Optional persistent cache for extracted page text.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(crate) const CACHE_IO_CONCURRENCY: usize = 4;
const PRUNE_SCAN_LIMIT: usize = 4_096;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::search::KestrelError;

// Bump when extraction semantics or the persistent identity contract changes.
const EXTRACTION_CACHE_VERSION: &str = "page-text-v3";

#[derive(Deserialize, Serialize)]
struct CachedPage {
    version: String,
    url: String,
    content_limit: usize,
    max_response_bytes: usize,
    created_at: SystemTime,
    content_sha256: String,
    content: String,
}

/// A TTL-bound disk cache keyed by conservative request URL, extraction version and limit.
#[derive(Clone, Debug)]
pub struct PageCache {
    directory: PathBuf,
    ttl: Duration,
    max_entries: usize,
    io: Arc<Semaphore>,
    max_response_bytes: usize,
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
            max_response_bytes: crate::fetcher::DEFAULT_MAX_RESPONSE_BYTES,
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

    pub(crate) fn for_response_limit(&self, limit: usize) -> Self {
        let mut cache = self.clone();
        cache.max_response_bytes = limit;
        cache
    }

    pub(crate) async fn get(&self, url: &str, content_limit: usize) -> Option<String> {
        let target = self.target(url, content_limit);
        let expected_url = request_url(url);
        let ttl = self.ttl;
        let response_limit = self.max_response_bytes;
        self.run_io(move || {
            let file = match std::fs::File::open(target) {
                Ok(file) => file,
                Err(_) => return Ok(None),
            };
            // JSON escaping uses at most six bytes per scalar, plus bounded metadata.
            let max_bytes = content_limit.saturating_mul(6).saturating_add(65_536);
            if file.metadata()?.len() > max_bytes as u64 {
                return Ok(None);
            }
            let mut bytes = Vec::new();
            file.take((max_bytes as u64).saturating_add(1))
                .read_to_end(&mut bytes)?;
            if bytes.len() > max_bytes {
                return Ok(None);
            }
            let Ok(page) = serde_json::from_slice::<CachedPage>(&bytes) else {
                return Ok(None);
            };
            if page.version != EXTRACTION_CACHE_VERSION
                || page.url != expected_url
                || page.content_limit != content_limit
                || page.max_response_bytes != response_limit
                || page.content.chars().count() > content_limit
                || page.content_sha256 != format!("{:x}", Sha256::digest(page.content.as_bytes()))
                || !SystemTime::now()
                    .duration_since(page.created_at)
                    .is_ok_and(|age| age <= ttl)
            {
                return Ok(None);
            }
            Ok(Some(page.content))
        })
        .await
        .ok()
        .flatten()
    }

    // Admission happens before spawning, and the worker owns the permit. Dropping
    // an async caller cannot admit unlimited replacement work behind a blocked disk.
    pub(crate) async fn run_io<T: Send + 'static>(
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
        let mut page = CachedPage {
            version: EXTRACTION_CACHE_VERSION.into(),
            url: request_url(url),
            content_limit,
            max_response_bytes: self.max_response_bytes,
            created_at: SystemTime::now(),
            content_sha256: String::new(),
            content: content.to_owned(),
        };
        self.run_io(move || {
            page.content_sha256 = format!("{:x}", Sha256::digest(page.content.as_bytes()));
            std::fs::create_dir_all(&cache.directory)?;
            let _lock = cache.lock()?;
            atomic_json(&cache.directory, &target, &page)?;
            Ok(())
        })
        .await
    }

    pub(crate) fn lock(&self) -> Result<std::fs::File, std::io::Error> {
        let lock = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join(".page-cache.lock"))?;
        let started = std::time::Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(std::fs::TryLockError::WouldBlock)
                    if started.elapsed() < Duration::from_millis(250) =>
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "page cache lock deadline",
                    ));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error),
            }
        }
    }

    pub(crate) async fn prune(&self) -> Result<(), KestrelError> {
        let cache = self.clone();
        self.run_io(move || {
            let _lock = cache.lock()?;
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
        // Search deduplication is a heuristic, not proof of HTTP resource identity.
        // Fragments are not sent to the server. Preserve query order, tracking
        // parameters, encoded paths and trailing slashes. URL parsing applies only
        // standard URL normalization (for example default ports and host case).
        let key_url = request_url(url);
        let digest = Sha256::digest(
            format!(
                "{EXTRACTION_CACHE_VERSION}\0{key_url}\0{content_limit}\0{}",
                self.max_response_bytes
            )
            .as_bytes(),
        );
        self.directory.join(format!("{digest:x}.txt"))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

fn request_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => url.to_owned(),
    }
}

const PAGE_QUEUE_ENTRIES: usize = 16;
const PAGE_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const STORAGE_WAIT: Duration = Duration::from_millis(250);

struct PageWrite {
    url: String,
    content: String,
    _memory: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) struct PageWriteQueue {
    sender: tokio::sync::mpsc::Sender<PageWrite>,
    memory: Arc<Semaphore>,
    drain: Arc<std::sync::Mutex<Option<tokio::time::Instant>>>,
}

impl Drop for PageWriteQueue {
    fn drop(&mut self) {
        if let Ok(mut end) = self.drain.lock() {
            *end = Some(tokio::time::Instant::now() + STORAGE_WAIT);
        }
    }
}

impl PageWriteQueue {
    pub(crate) async fn enqueue(
        &self,
        url: &str,
        content: &str,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<(), ()> {
        let bytes = content.len().saturating_add(url.len()).max(1);
        if bytes > PAGE_QUEUE_BYTES || url.len() > 8_192 {
            eprintln!(
                "[kestrel] Page cache skipped an oversized entry; extracted text is retained."
            );
            return Ok(());
        }
        let memory = crate::numeric::before_deadline(
            deadline,
            self.memory.clone().acquire_many_owned(bytes as u32),
        )
        .await?
        .map_err(|_| ())?;
        crate::numeric::before_deadline(
            deadline,
            self.sender.send(PageWrite {
                url: url.to_owned(),
                content: content.to_owned(),
                _memory: memory,
            }),
        )
        .await?
        .map_err(|_| ())
    }
}

pub(crate) fn page_writer<'a>(
    cache: Option<&'a PageCache>,
    content_limit: usize,
    deadline: Option<tokio::time::Instant>,
    cancellation: Option<&'a crate::SearchRecovery>,
) -> (
    Option<PageWriteQueue>,
    impl std::future::Future<Output = bool> + 'a,
) {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<PageWrite>(PAGE_QUEUE_ENTRIES);
    let drain = Arc::new(std::sync::Mutex::new(None));
    let queue = cache.map(|_| PageWriteQueue {
        sender,
        memory: Arc::new(Semaphore::new(PAGE_QUEUE_BYTES)),
        drain: drain.clone(),
    });
    let writer = async move {
        let Some(cache) = cache else { return false };
        let mut committed = 0;
        let mut failed = 0;
        let mut exhausted = false;
        while let Some(page) = receiver.recv().await {
            let now = tokio::time::Instant::now();
            let drain_end = drain.lock().ok().and_then(|end| *end);
            let end = deadline.or_else(|| {
                Some(drain_end.map_or(now + STORAGE_WAIT, |end| end.min(now + STORAGE_WAIT)))
            });
            let end = if cancellation.is_some_and(|s| s.is_cancelled()) {
                let cap = drain_end.unwrap_or(now + STORAGE_WAIT);
                Some(end.map_or(cap, |end| end.min(cap)))
            } else {
                end
            };
            match crate::recovery::storage_wait(
                end,
                cancellation,
                cache.put(&page.url, content_limit, &page.content),
            )
            .await
            {
                Ok(Ok(())) => committed += 1,
                Ok(Err(error)) => {
                    failed += 1;
                    eprintln!("[kestrel] Page cache commit failed: {error}");
                }
                Err(()) => {
                    failed += 1;
                    exhausted |= deadline.is_some_and(|end| tokio::time::Instant::now() >= end);
                    eprintln!(
                        "[kestrel] Page cache storage wait expired; extracted text is retained."
                    );
                }
            }
            if deadline.is_some_and(|end| tokio::time::Instant::now() >= end)
                || (deadline.is_none()
                    && drain_end.is_some_and(|end| tokio::time::Instant::now() >= end))
            {
                receiver.close();
                while receiver.try_recv().is_ok() {
                    failed += 1;
                }
                break;
            }
        }
        if committed > 0 {
            let drain_end = drain.lock().ok().and_then(|end| *end);
            let now = tokio::time::Instant::now();
            let end = deadline.or_else(|| {
                Some(drain_end.map_or(now + STORAGE_WAIT, |end| end.min(now + STORAGE_WAIT)))
            });
            let end = if cancellation.is_some_and(|s| s.is_cancelled()) {
                let cap = drain_end.unwrap_or(now + STORAGE_WAIT);
                Some(end.map_or(cap, |end| end.min(cap)))
            } else {
                end
            };
            match crate::recovery::storage_wait(end, cancellation, cache.prune()).await {
                Ok(Ok(())) => (),
                Ok(Err(error)) => eprintln!("[kestrel] Page cache maintenance failed: {error}"),
                Err(()) => {
                    exhausted |= deadline.is_some_and(|end| tokio::time::Instant::now() >= end)
                }
            }
        }
        if committed > 0 || failed > 0 {
            eprintln!(
                "[kestrel] Page cache: {committed} committed, {failed} uncommitted; accepted text alone is not a commit."
            );
        }
        exhausted
    };
    (queue, writer)
}

pub(crate) fn atomic_json(
    directory: &Path,
    target: &Path,
    value: &impl Serialize,
) -> std::io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    {
        let mut writer = std::io::BufWriter::new(temporary.as_file_mut());
        serde_json::to_writer(&mut writer, value).map_err(std::io::Error::other)?;
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
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
    #[tokio::test]
    async fn damaged_expired_and_incompatible_entries_are_misses() {
        let directory = tempfile::tempdir().unwrap();
        let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        let url = "https://example.com/page";
        cache.put(url, 100, "complete text").await.unwrap();
        assert_eq!(cache.get(url, 100).await.as_deref(), Some("complete text"));
        assert!(cache.get(url, 200).await.is_none());
        assert!(cache.for_response_limit(500).get(url, 100).await.is_none());
        let target = cache.target(url, 100);
        let original = std::fs::read(&target).unwrap();
        for field in [
            "content",
            "version",
            "url",
            "created_at",
            "max_response_bytes",
        ] {
            let mut page: serde_json::Value = serde_json::from_slice(&original).unwrap();
            page[field] = match field {
                "created_at" => serde_json::to_value(SystemTime::UNIX_EPOCH).unwrap(),
                "max_response_bytes" => serde_json::json!(1),
                _ => serde_json::json!("damaged"),
            };
            std::fs::write(&target, serde_json::to_vec(&page).unwrap()).unwrap();
            assert!(
                cache.get(url, 100).await.is_none(),
                "accepted damaged {field}"
            );
        }
        std::fs::write(&target, &original[..original.len() / 2]).unwrap();
        assert!(cache.get(url, 100).await.is_none());
        std::fs::write(&target, vec![b'x'; 70_000]).unwrap();
        assert!(cache.get(url, 100).await.is_none());
        // Interrupted temporary files are never replayed as entries.
        std::fs::write(directory.path().join(".tmp-interrupted"), &original).unwrap();
        assert!(cache.get(url, 100).await.is_none());
    }

    #[tokio::test]
    async fn concurrent_handles_commit_whole_entries_and_release_locks() {
        let directory = tempfile::tempdir().unwrap();
        let first = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        let second = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
        let url = "https://example.com/shared";
        let (a, b) = tokio::join!(
            first.put(url, 100, "first complete record"),
            second.put(url, 100, "second complete record")
        );
        a.unwrap();
        b.unwrap();
        let result = first.get(url, 100).await.unwrap();
        assert!(result == "first complete record" || result == "second complete record");
        let lock = first.lock().unwrap();
        assert!(second.put(url, 100, "blocked").await.is_err());
        drop(lock);
        second.put(url, 100, "after lock release").await.unwrap();
        assert_eq!(
            first.get(url, 100).await.as_deref(),
            Some("after lock release")
        );
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
