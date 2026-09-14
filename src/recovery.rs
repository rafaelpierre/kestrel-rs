//! Opt-in, bounded persistence of accepted provider snapshots.
use crate::search::KestrelError;
use crate::{
    Engine, SearchResult, TimeFilter,
    cache::{PageCache, atomic_json},
    numeric::before_deadline,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::{Semaphore, mpsc};

const VERSION: &str = "provider-progress-v1/adapter-v1";
const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
const QUEUE_BYTES: usize = 16 * MAX_RECORD_BYTES;
const WAIT: Duration = Duration::from_millis(250);

/// Independent opt-in provider progress store. Page text uses `PageCache` instead.
#[derive(Clone, Debug)]
pub struct SearchRecovery {
    disk: PageCache,
    ttl: Duration,
    cancel: tokio::sync::watch::Sender<bool>,
}
impl SearchRecovery {
    /// Create configuration without touching disk. TTL must be positive.
    pub fn new(directory: impl Into<PathBuf>, ttl: Duration) -> Result<Self, KestrelError> {
        Ok(Self {
            disk: PageCache::new(directory, ttl)?,
            ttl,
            cancel: tokio::sync::watch::channel(false).0,
        })
    }
    /// Set the best-effort number of retained provider units (default 1,000).
    pub fn with_max_entries(mut self, count: usize) -> Result<Self, KestrelError> {
        self.disk = self.disk.with_max_entries(count)?;
        Ok(self)
    }
    /// Default CLI location, separate from extracted page text.
    pub fn default_directory() -> Result<PathBuf, KestrelError> {
        home::home_dir()
            .map(|p| p.join(".cache/kestrel/search-v1"))
            .ok_or_else(|| KestrelError::InvalidRequest("home directory is unavailable".into()))
    }
    /// Request bounded graceful cancellation of operations using this store.
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
    /// Whether graceful cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        *self.cancel.borrow()
    }
    pub(crate) async fn cancelled(&self) {
        let mut receiver = self.cancel.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
    fn target(&self, key: &UnitKey) -> Result<PathBuf, std::io::Error> {
        let bytes = serde_json::to_vec(key).map_err(std::io::Error::other)?;
        Ok(self
            .disk
            .directory()
            .join(format!("{:x}.txt", Sha256::digest(bytes))))
    }
    async fn commit(&self, mut snapshot: Snapshot) -> Result<bool, KestrelError> {
        let disk = self.disk.clone();
        let target = self.target(&snapshot.key)?;
        let ttl = self.ttl;
        self.disk
            .run_io(move || {
                snapshot.checksum.clear();
                let bytes = serde_json::to_vec(&snapshot).map_err(std::io::Error::other)?;
                snapshot.checksum = format!("{:x}", Sha256::digest(bytes));
                if serde_json::to_vec(&snapshot)
                    .map_err(std::io::Error::other)?
                    .len()
                    > MAX_RECORD_BYTES
                {
                    return Err(std::io::Error::other("provider snapshot exceeds 4 MiB"));
                }
                std::fs::create_dir_all(disk.directory())?;
                let _lock = disk.lock()?;
                if let Some(previous) = read_snapshot(&target)
                    && previous.key == snapshot.key
                    && SystemTime::now()
                        .duration_since(previous.updated_at)
                        .is_ok_and(|age| age <= ttl)
                    && (previous.generation > snapshot.generation
                        || (previous.generation == snapshot.generation
                            && previous.sequence >= snapshot.sequence))
                {
                    return Ok(false);
                }
                atomic_json(disk.directory(), &target, &snapshot)?;
                Ok(true)
            })
            .await
    }
    pub(crate) async fn restore(&self, key: &UnitKey) -> Result<Snapshot, &'static str> {
        let target = self.target(key).map_err(|_| "invalid identity")?;
        let key = key.clone();
        let ttl = self.ttl;
        self.disk
            .run_io(move || {
                match std::fs::metadata(&target) {
                    Err(error) => {
                        return Ok(Err(if error.kind() == std::io::ErrorKind::NotFound {
                            "evicted or absent"
                        } else {
                            "storage read failure"
                        }));
                    }
                    Ok(meta) if meta.len() > MAX_RECORD_BYTES as u64 => {
                        return Ok(Err("oversized entry"));
                    }
                    Ok(_) => (),
                }
                let Some(snapshot) = read_snapshot(&target) else {
                    return Ok(Err("corrupt or incompatible entry"));
                };
                if snapshot.key != key {
                    return Ok(Err("incompatible identity"));
                }
                if snapshot.state == State::Invalid {
                    return Ok(Err("invalidated provider attempt"));
                }
                if !SystemTime::now()
                    .duration_since(snapshot.updated_at)
                    .is_ok_and(|age| age <= ttl)
                {
                    return Ok(Err("expired or future-dated entry"));
                }
                Ok(Ok(snapshot))
            })
            .await
            .map_err(|_| "storage read failure")?
    }
    #[cfg(test)]
    pub(crate) async fn load(&self, key: &UnitKey) -> Option<Snapshot> {
        self.restore(key).await.ok()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct UnitKey {
    version: String,
    query: String,
    engine: Engine,
    region: String,
    time_filter: TimeFilter,
    coverage: String,
}
impl UnitKey {
    pub(crate) fn new(query: &str, engine: Engine, region: &str, time_filter: TimeFilter) -> Self {
        Self {
            version: VERSION.into(),
            query: query.into(),
            engine,
            region: region.into(),
            time_filter,
            coverage: {
                let base = "built-in-endpoints/first-response/provider-limits-v1".to_owned();
                #[cfg(feature = "test-fixtures")]
                let base = std::env::var("KESTREL_TEST_PROVIDER_ENDPOINT")
                    .map_or(base.clone(), |endpoint| {
                        format!("{base}/fixture:{endpoint}")
                    });
                base
            },
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Incomplete,
    Complete,
    Invalid,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Snapshot {
    key: UnitKey,
    generation: (SystemTime, String),
    sequence: u64,
    updated_at: SystemTime,
    pub(crate) state: State,
    pub(crate) records: Vec<SearchResult>,
    checksum: String,
}
fn read_snapshot(path: &std::path::Path) -> Option<Snapshot> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_RECORD_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_RECORD_BYTES {
        return None;
    }
    let mut snapshot: Snapshot = serde_json::from_slice(&bytes).ok()?;
    let checksum = std::mem::take(&mut snapshot.checksum);
    let records = serde_json::to_vec(&snapshot).ok()?;
    snapshot.checksum = checksum;
    (snapshot.key.version == VERSION
        && snapshot.checksum == format!("{:x}", Sha256::digest(records))
        && (snapshot.state != State::Invalid || snapshot.records.is_empty())
        && snapshot.records.iter().all(|r| {
            r.engine == Some(snapshot.key.engine)
                && r.query.as_deref() == Some(snapshot.key.query.as_str())
                && r.engine_rank.is_some_and(|rank| rank > 0)
                && r.content.is_none()
                && r.sources.iter().all(|p| {
                    p.engine == snapshot.key.engine && p.query == snapshot.key.query && p.rank > 0
                })
        }))
    .then_some(snapshot)
}

struct Queued {
    snapshot: Snapshot,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
#[derive(Clone)]
pub(crate) struct ProgressQueue {
    sender: mpsc::Sender<Queued>,
    pub(crate) store: SearchRecovery,
    memory: Arc<Semaphore>,
    generation: (SystemTime, String),
    drain: Arc<std::sync::Mutex<Option<tokio::time::Instant>>>,
}
impl Drop for ProgressQueue {
    fn drop(&mut self) {
        if self.sender.strong_count() == 1
            && let Ok(mut end) = self.drain.lock()
        {
            *end = Some(tokio::time::Instant::now() + WAIT);
        }
    }
}
impl ProgressQueue {
    pub(crate) async fn enqueue(
        &self,
        key: UnitKey,
        sequence: u64,
        state: State,
        records: &[SearchResult],
        deadline: Option<tokio::time::Instant>,
    ) {
        tokio::select! {
            () = self.store.cancelled() => eprintln!("[kestrel] Recovery admission closed by cancellation."),
            () = self.enqueue_inner(key, sequence, state, records, deadline) => (),
        }
    }
    async fn enqueue_inner(
        &self,
        key: UnitKey,
        sequence: u64,
        state: State,
        records: &[SearchResult],
        deadline: Option<tokio::time::Instant>,
    ) {
        // Conservative upper bound for escaped strings and per-record JSON metadata.
        let bytes = records.iter().fold(
            4096usize + key.query.len().saturating_mul(6) + key.region.len().saturating_mul(6),
            |n, r| {
                let strings = r
                    .title
                    .len()
                    .saturating_add(r.url.len())
                    .saturating_add(r.display_url.len())
                    .saturating_add(r.snippet.len())
                    .saturating_add(r.query.as_ref().map_or(0, String::len))
                    .saturating_add(
                        r.sources
                            .iter()
                            .map(|p| p.query.len().saturating_add(128))
                            .sum::<usize>(),
                    );
                n.saturating_add(strings.saturating_mul(6))
                    .saturating_add(1024)
            },
        );
        if bytes > MAX_RECORD_BYTES {
            eprintln!(
                "[kestrel] Recovery skipped oversized snapshot; accepted records are not committed."
            );
            return;
        }
        let permit = match before_deadline(
            deadline,
            self.memory.clone().acquire_many_owned(bytes as u32),
        )
        .await
        {
            Ok(Ok(p)) => p,
            _ => {
                eprintln!("[kestrel] Recovery queue deadline; accepted records are not committed.");
                return;
            }
        };
        let snapshot = Snapshot {
            key,
            generation: self.generation.clone(),
            sequence,
            updated_at: SystemTime::now(),
            state,
            records: records.to_vec(),
            checksum: String::new(),
        };
        if !matches!(
            before_deadline(
                deadline,
                self.sender.send(Queued {
                    snapshot,
                    _permit: permit
                })
            )
            .await,
            Ok(Ok(()))
        ) {
            eprintln!(
                "[kestrel] Recovery queue closed or deadline reached; accepted records are not committed."
            );
        }
    }
}

// Cancellation can arrive after a storage syscall starts. Continue polling that
// same operation only within the smaller of its existing deadline and drain cap.
pub(crate) async fn storage_wait<T>(
    deadline: Option<tokio::time::Instant>,
    cancellation: Option<&SearchRecovery>,
    work: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    let Some(store) = cancellation else {
        return before_deadline(deadline, work).await;
    };
    tokio::pin!(work);
    if !store.is_cancelled() {
        tokio::select! {
            result = before_deadline(deadline,&mut work) => return result,
            () = store.cancelled() => (),
        }
    }
    let cap = tokio::time::Instant::now() + WAIT;
    before_deadline(Some(deadline.map_or(cap, |end| end.min(cap))), &mut work).await
}

pub(crate) fn writer(
    store: Option<&SearchRecovery>,
    deadline: Option<tokio::time::Instant>,
) -> (
    Option<ProgressQueue>,
    impl std::future::Future<Output = ()> + '_,
) {
    let (sender, mut receiver) = mpsc::channel::<Queued>(16);
    let drain = Arc::new(std::sync::Mutex::new(None));
    let queue = store.map(|store| ProgressQueue {
        store: store.clone(),
        drain: drain.clone(),
        sender,
        memory: Arc::new(Semaphore::new(QUEUE_BYTES)),
        generation: (SystemTime::now(), uuid::Uuid::new_v4().to_string()),
    });
    let writer = async move {
        let Some(store) = store else { return };
        let mut committed = 0;
        let mut lost = 0;
        while let Some(item) = receiver.recv().await {
            let end = deadline.unwrap_or_else(|| {
                let end = tokio::time::Instant::now() + WAIT;
                drain
                    .lock()
                    .ok()
                    .and_then(|d| *d)
                    .map_or(end, |d| d.min(end))
            });
            let end = if store.is_cancelled() {
                let cap = drain
                    .lock()
                    .ok()
                    .and_then(|d| *d)
                    .unwrap_or_else(|| tokio::time::Instant::now() + WAIT);
                end.min(cap)
            } else {
                end
            };
            match storage_wait(Some(end), Some(store), store.commit(item.snapshot)).await {
                Ok(Ok(true)) => committed += 1,
                Ok(Ok(false)) => {
                    eprintln!("[kestrel] Recovery ignored a superseded generation or sequence.")
                }
                Ok(Err(error)) => {
                    lost += 1;
                    eprintln!("[kestrel] Recovery commit failed: {error}");
                }
                Err(()) => {
                    lost += 1;
                    eprintln!("[kestrel] Recovery storage deadline; commit unacknowledged.");
                }
            }
            if tokio::time::Instant::now() >= end {
                receiver.close();
                while receiver.try_recv().is_ok() {
                    lost += 1;
                }
                break;
            }
        }
        if committed > 0 {
            let end = deadline.unwrap_or_else(|| {
                let end = tokio::time::Instant::now() + WAIT;
                drain
                    .lock()
                    .ok()
                    .and_then(|d| *d)
                    .map_or(end, |d| d.min(end))
            });
            let end = if store.is_cancelled() {
                let cap = drain
                    .lock()
                    .ok()
                    .and_then(|d| *d)
                    .unwrap_or_else(|| tokio::time::Instant::now() + WAIT);
                end.min(cap)
            } else {
                end
            };
            if !matches!(
                storage_wait(Some(end), Some(store), store.disk.prune()).await,
                Ok(Ok(()))
            ) {
                eprintln!("[kestrel] Recovery maintenance failed or timed out.");
            }
        }
        eprintln!("[kestrel] Recovery: {committed} snapshots committed, {lost} unacknowledged.");
    };
    (queue, writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> UnitKey {
        UnitKey::new("fixture", Engine::Bing, "", TimeFilter::Any)
    }
    fn record() -> SearchResult {
        SearchResult {
            title: "record".into(),
            url: "https://example.org/record".into(),
            display_url: String::new(),
            snippet: "evidence".into(),
            content: None,
            bm25_score: None,
            engine: Some(Engine::Bing),
            query: Some("fixture".into()),
            engine_rank: Some(2),
            sources: vec![crate::SourceOccurrence {
                engine: Engine::Bing,
                query: "fixture".into(),
                rank: 2,
            }],
        }
    }
    fn snapshot(generation: (SystemTime, String), sequence: u64, state: State) -> Snapshot {
        Snapshot {
            key: key(),
            generation,
            sequence,
            updated_at: SystemTime::now(),
            state,
            records: if state == State::Invalid {
                vec![]
            } else {
                vec![record()]
            },
            checksum: String::new(),
        }
    }
    #[tokio::test]
    async fn cancellation_bounds_an_already_pending_storage_wait() {
        let dir = tempfile::tempdir().unwrap();
        let store = SearchRecovery::new(dir.path(), Duration::from_secs(60)).unwrap();
        let started = tokio::time::Instant::now();
        let wait = storage_wait(
            Some(started + Duration::from_secs(20)),
            Some(&store),
            std::future::pending::<()>(),
        );
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            store.cancel();
        };
        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(1), async { tokio::join!(wait, cancel) })
                .await
                .unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn recovered_client_futures_remain_send() {
        fn assert_send<T: Send>(_: T) {}
        let directory = tempfile::tempdir().unwrap();
        let store = SearchRecovery::new(directory.path(), Duration::from_secs(60)).unwrap();
        let client = crate::KestrelClient::new().unwrap().with_recovery(store);
        let queries = vec!["fixture".to_owned()];
        let options = crate::SearchOptions::default();
        assert_send(client.search_many_detailed(&queries, &options));
    }
    #[tokio::test]
    async fn snapshot_replacement_completion_retraction_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let store = SearchRecovery::new(dir.path(), Duration::from_secs(60)).unwrap();
        let generation = (SystemTime::now(), "a".into());
        store
            .commit(snapshot(generation.clone(), 1, State::Incomplete))
            .await
            .unwrap();
        assert_eq!(store.load(&key()).await.unwrap().state, State::Incomplete);
        let mut final_record = snapshot(generation.clone(), 2, State::Complete);
        final_record.records[0].title = "replacement".into();
        store.commit(final_record).await.unwrap();
        assert!(
            !store
                .commit(snapshot(generation.clone(), 1, State::Incomplete))
                .await
                .unwrap()
        );
        let restored = store.load(&key()).await.unwrap();
        assert_eq!(restored.state, State::Complete);
        assert_eq!(restored.records[0].title, "replacement");
        assert_eq!(restored.records[0].sources[0].rank, 2);
        store
            .commit(snapshot(generation.clone(), 3, State::Invalid))
            .await
            .unwrap();
        assert!(store.load(&key()).await.is_none());
        store
            .commit(snapshot(generation, 4, State::Incomplete))
            .await
            .unwrap();
        let target = store.target(&key()).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        let mut damaged: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        damaged["state"] = serde_json::json!("complete");
        std::fs::write(&target, serde_json::to_vec(&damaged).unwrap()).unwrap();
        assert!(
            store.load(&key()).await.is_none(),
            "completion must be checksummed with records"
        );
        std::fs::write(&target, &bytes[..bytes.len() / 2]).unwrap();
        assert!(store.load(&key()).await.is_none());
    }
    #[tokio::test]
    async fn generation_order_ttl_and_concurrent_writers() {
        let dir = tempfile::tempdir().unwrap();
        let first = SearchRecovery::new(dir.path(), Duration::from_secs(60)).unwrap();
        let second = SearchRecovery::new(dir.path(), Duration::from_secs(60)).unwrap();
        let old = (SystemTime::UNIX_EPOCH, "a".into());
        let new = (SystemTime::now(), "b".into());
        let (a, b) = tokio::join!(
            first.commit(snapshot(old.clone(), 99, State::Incomplete)),
            second.commit(snapshot(new.clone(), 1, State::Complete))
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(first.load(&key()).await.unwrap().state, State::Complete);
        assert!(
            !first
                .commit(snapshot(old, 100, State::Invalid))
                .await
                .unwrap()
        );
        let mut expired = snapshot(new, 2, State::Complete);
        expired.updated_at = SystemTime::UNIX_EPOCH;
        first.commit(expired).await.unwrap();
        assert!(first.load(&key()).await.is_none());
        let changed = UnitKey::new("fixture", Engine::Bing, "gb", TimeFilter::Any);
        assert!(first.load(&changed).await.is_none());
    }
    #[tokio::test]
    async fn queue_backpressure_is_bounded_and_cancellable() {
        let dir = tempfile::tempdir().unwrap();
        let store = SearchRecovery::new(dir.path(), Duration::from_secs(60)).unwrap();
        let (queue, writer) = writer(Some(&store), None);
        let queue = queue.unwrap();
        // Do not poll the writer: the seventeenth send must await capacity.
        for seq in 1..=16 {
            queue
                .enqueue(key(), seq, State::Incomplete, &[record()], None)
                .await;
        }
        assert_eq!(queue.sender.capacity(), 0);
        let admitted_memory = queue.memory.available_permits();
        assert!(admitted_memory < QUEUE_BYTES);
        {
            let records = [record()];
            let pending = queue.enqueue(key(), 17, State::Complete, &records, None);
            tokio::pin!(pending);
            assert!(futures_util::poll!(&mut pending).is_pending());
            // A blocked send holds a byte permit; dropping it must return that permit.
            assert!(queue.memory.available_permits() < admitted_memory);
        }
        assert_eq!(queue.memory.available_permits(), admitted_memory);
        let limit = tokio::time::Instant::now() + Duration::from_millis(20);
        queue
            .enqueue(key(), 17, State::Complete, &[record()], Some(limit))
            .await;
        assert_eq!(queue.sender.capacity(), 0);
        assert_eq!(queue.memory.available_permits(), admitted_memory);
        {
            let records = [record()];
            let pending = queue.enqueue(key(), 18, State::Complete, &records, None);
            tokio::pin!(pending);
            assert!(futures_util::poll!(&mut pending).is_pending());
            store.cancel();
            assert!(futures_util::poll!(&mut pending).is_ready());
        }
        assert_eq!(queue.sender.capacity(), 0);
        assert_eq!(queue.memory.available_permits(), admitted_memory);

        // Admission is not persistence. Never poll the writer in this queue test:
        // dropping its receiver must release all queued memory without disk I/O.
        let memory = queue.memory.clone();
        drop(queue);
        drop(writer);
        assert_eq!(memory.available_permits(), QUEUE_BYTES);
        assert!(!store.target(&key()).unwrap().exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queue_drain_deadline_releases_uncommitted_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let store = SearchRecovery::new(dir.path(), Duration::from_secs(60)).unwrap();
        // Occupy every storage worker until explicitly released. This reproduces
        // storage contention without relying on the host's filesystem speed.
        let mut releases = Vec::new();
        let mut workers = Vec::new();
        for _ in 0..crate::cache::CACHE_IO_CONCURRENCY {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel::<()>();
            releases.push(release);
            let disk = store.disk.clone();
            workers.push(tokio::spawn(async move {
                disk.run_io(move || {
                    let _ = started.send(());
                    let _ = wait.recv();
                    Ok(())
                })
                .await
                .unwrap();
            }));
            ready.await.unwrap();
        }
        let (queue, writer) = writer(Some(&store), None);
        let queue = queue.unwrap();
        for seq in 1..=16 {
            queue
                .enqueue(key(), seq, State::Incomplete, &[record()], None)
                .await;
        }
        let memory = queue.memory.clone();
        drop(queue);
        // The production drain cap must finish even while storage remains busy.
        // This outer timeout is only a deadlock guard, not a disk-speed assertion.
        let drained = tokio::time::timeout(Duration::from_secs(5), writer).await;
        drop(releases);
        for worker in workers {
            worker.await.unwrap();
        }
        drained.unwrap();
        assert_eq!(memory.available_permits(), QUEUE_BYTES);
        assert!(store.load(&key()).await.is_none());
        assert!(!store.target(&key()).unwrap().exists());
    }
}
