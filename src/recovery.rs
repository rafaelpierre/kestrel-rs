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
}
impl SearchRecovery {
    /// Create configuration without touching disk. TTL must be positive.
    pub fn new(directory: impl Into<PathBuf>, ttl: Duration) -> Result<Self, KestrelError> {
        Ok(Self {
            disk: PageCache::new(directory, ttl)?,
            ttl,
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
    #[cfg(test)]
    pub(crate) async fn load(&self, key: &UnitKey) -> Option<Snapshot> {
        let target = self.target(key).ok()?;
        let key = key.clone();
        let ttl = self.ttl;
        self.disk
            .run_io(move || {
                Ok(read_snapshot(&target).filter(|s| {
                    s.key == key
                        && s.key.version == VERSION
                        && s.state != State::Invalid
                        && SystemTime::now()
                            .duration_since(s.updated_at)
                            .is_ok_and(|age| age <= ttl)
                }))
            })
            .await
            .ok()
            .flatten()
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
            coverage: "built-in-endpoints/first-response/provider-limits-v1".into(),
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

pub(crate) fn writer(
    store: Option<&SearchRecovery>,
    deadline: Option<tokio::time::Instant>,
) -> (
    Option<ProgressQueue>,
    impl std::future::Future<Output = ()> + '_,
) {
    let (sender, mut receiver) = mpsc::channel::<Queued>(16);
    let drain = Arc::new(std::sync::Mutex::new(None));
    let queue = store.map(|_| ProgressQueue {
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
            match before_deadline(Some(end), store.commit(item.snapshot)).await {
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
            if !matches!(
                before_deadline(Some(end), store.disk.prune()).await,
                Ok(Ok(()))
            ) {
                eprintln!("[kestrel] Recovery maintenance failed or timed out.");
            }
        }
        eprintln!(
            "[kestrel] Recovery: {committed} snapshots committed, {lost} unacknowledged; replay is not enabled in this slice."
        );
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
        let limit = tokio::time::Instant::now() + Duration::from_millis(20);
        queue
            .enqueue(key(), 17, State::Complete, &[record()], Some(limit))
            .await;
        assert_eq!(queue.sender.capacity(), 0);
        assert!(queue.memory.available_permits() < QUEUE_BYTES);
        drop(queue);
        writer.await;
        assert_eq!(store.load(&key()).await.unwrap().sequence, 16);
    }
}
