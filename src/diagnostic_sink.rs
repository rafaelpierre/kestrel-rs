//! Bounded best-effort persistence, independent of the async executor.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

/// Process-wide local diagnostic policy. Configure before the first event or trace.
#[derive(Clone, Debug)]
pub struct Config {
    /// Disable all local event and provider-trace persistence (not OTLP or artifacts).
    pub enabled: bool,
    /// Maximum records being prepared or queued (1..=65536), excluding the active write.
    pub queue_capacity: usize,
    /// Maximum retained payload bytes, including preparation and active writes.
    pub max_pending_bytes: usize,
    /// Maximum combined payload bytes in one record; oversized records are dropped.
    pub max_record_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            queue_capacity: 64,
            max_pending_bytes: 16 * 1024 * 1024,
            max_record_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Cumulative process-local counters. A successful flush does not erase losses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Records rejected by capacity, byte limits, serialization or writer failure.
    pub dropped: u64,
    /// Accepted records for which at least one filesystem operation failed.
    pub write_failures: u64,
    /// Payload bytes currently being prepared, queued or written.
    pub pending_bytes: usize,
}

#[derive(Default)]
struct Counters {
    bytes: AtomicUsize,
    dropped: AtomicU64,
    failures: AtomicU64,
}

struct Payload {
    bytes: Vec<u8>,
    counters: Arc<Counters>,
    total_limit: usize,
    limit: usize,
}

impl Write for Payload {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("diagnostic record byte limit"));
        }
        self.counters
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_add(bytes.len())
                    .filter(|n| *n <= self.total_limit)
            })
            .map_err(|_| io::Error::other("diagnostic pending byte limit"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.counters
            .bytes
            .fetch_sub(self.bytes.len(), Ordering::Relaxed);
    }
}

struct FileRecord {
    path: PathBuf,
    append: bool,
    payload: Payload,
}

/// One admission unit, containing at most the raw response and its metadata.
pub(crate) struct Record {
    files: Vec<FileRecord>,
    config: Config,
    counters: Arc<Counters>,
    used: usize,
}

impl Record {
    pub(crate) fn file(
        &mut self,
        path: PathBuf,
        append: bool,
        encode: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.files.len() == 2 {
            return Err(io::Error::other("diagnostic file count limit"));
        }
        let mut payload = Payload {
            bytes: Vec::new(),
            counters: Arc::clone(&self.counters),
            total_limit: self.config.max_pending_bytes,
            limit: self.config.max_record_bytes.saturating_sub(self.used),
        };
        encode(&mut payload)?;
        self.used += payload.bytes.len();
        self.files.push(FileRecord {
            path,
            append,
            payload,
        });
        Ok(())
    }

    fn persist(&self) -> io::Result<()> {
        for file in &self.files {
            if let Some(parent) = file.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = OpenOptions::new()
                .create(true)
                .write(true)
                .append(file.append)
                .truncate(!file.append)
                .open(&file.path)?;
            output.write_all(&file.payload.bytes)?;
        }
        Ok(())
    }
}

enum Message {
    Record(Record),
    Flush(oneshot::Sender<()>),
}

pub(crate) struct Sink {
    sender: mpsc::Sender<Message>,
    counters: Arc<Counters>,
    config: Config,
}

impl Sink {
    fn new(config: Config) -> io::Result<Self> {
        Self::with_writer(config, Record::persist)
    }

    fn with_writer(
        config: Config,
        write: impl Fn(&Record) -> io::Result<()> + Send + 'static,
    ) -> io::Result<Self> {
        let (sender, mut receiver) = mpsc::channel(config.queue_capacity);
        let counters = Arc::new(Counters::default());
        let worker_counters = Arc::clone(&counters);
        std::thread::Builder::new()
            .name("kestrel-diagnostics".into())
            .spawn(move || {
                while let Some(message) = receiver.blocking_recv() {
                    match message {
                        Message::Record(record) => {
                            if write(&record).is_err() {
                                worker_counters.failures.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Message::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        Ok(Self {
            sender,
            counters,
            config,
        })
    }

    pub(crate) fn submit(&self, encode: impl FnOnce(&mut Record) -> io::Result<()>) {
        // Reserve before encoding: concurrent producers cannot create an unbounded backlog.
        let Ok(permit) = self.sender.try_reserve() else {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let mut record = Record {
            files: Vec::new(),
            config: self.config.clone(),
            counters: Arc::clone(&self.counters),
            used: 0,
        };
        if encode(&mut record).is_err() {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        permit.send(Message::Record(record));
    }

    async fn flush(&self, timeout: Duration) -> bool {
        let Some(deadline) = tokio::time::Instant::now().checked_add(timeout) else {
            return false;
        };
        tokio::time::timeout_at(deadline, async {
            let (done, wait) = oneshot::channel();
            self.sender
                .send(Message::Flush(done))
                .await
                .map_err(|_| ())?;
            wait.await.map_err(|_| ())
        })
        .await
        .is_ok_and(|result| result.is_ok())
    }

    fn stats(&self) -> Stats {
        Stats {
            dropped: self.counters.dropped.load(Ordering::Relaxed),
            write_failures: self.counters.failures.load(Ordering::Relaxed),
            pending_bytes: self.counters.bytes.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_SINK: Sink;
}

static GLOBAL: OnceLock<io::Result<Option<Sink>>> = OnceLock::new();

/// Configure local diagnostics once, before any local logging/tracing use.
/// Returns an error for invalid limits, an already initialized sink or thread startup failure.
/// Disabling persistence does not disable returned diagnostics, OTLP or explicit artifacts.
pub fn configure(config: Config) -> io::Result<()> {
    if config.queue_capacity == 0
        || config.queue_capacity > 65536
        || config.max_record_bytes == 0
        || config.max_record_bytes > config.max_pending_bytes
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid diagnostic queue/byte limits",
        ));
    }
    // Store configuration before starting a worker so competing callers cannot spawn extras.
    let mut initialized = false;
    let result = GLOBAL.get_or_init(|| {
        initialized = true;
        if config.enabled {
            Sink::new(config).map(Some)
        } else {
            Ok(None)
        }
    });
    if !initialized {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "diagnostics already initialized",
        ));
    }
    result
        .as_ref()
        .map(|_| ())
        .map_err(|e| io::Error::new(e.kind(), e.to_string()))
}

fn global() -> Option<&'static Sink> {
    GLOBAL
        .get_or_init(|| Sink::new(Config::default()).map(Some))
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
}

pub(crate) fn submit(encode: impl FnOnce(&mut Record) -> io::Result<()>) {
    #[cfg(test)]
    if TEST_SINK.try_with(|_| ()).is_ok() {
        let _ = TEST_SINK.try_with(|sink| sink.submit(encode));
        return;
    }
    if let Some(sink) = global() {
        sink.submit(encode);
    }
}

/// Wait at most `timeout` for records enqueued before this flush barrier to finish.
/// Stop producers first to drain all records. Returns false on timeout or a stopped worker;
/// inspect [`stats`] for dropped records and filesystem errors. No fsync guarantee.
/// A timeout does not cancel a filesystem write. The single worker remains bounded.
/// The worker is process-lived; Drop never joins it and process exit can lose pending data.
pub async fn flush(timeout: Duration) -> bool {
    match GLOBAL.get() {
        None | Some(Ok(None)) => true,
        Some(Err(_)) => false,
        Some(Ok(Some(sink))) => sink.flush(timeout).await,
    }
}

/// Read counters without initializing diagnostics. Startup failure counts as one dropped record.
pub fn stats() -> Stats {
    match GLOBAL.get() {
        Some(Ok(Some(sink))) => sink.stats(),
        Some(Err(_)) => Stats {
            dropped: 1,
            ..Stats::default()
        },
        _ => Stats::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            queue_capacity: 2,
            max_pending_bytes: 32,
            max_record_bytes: 24,
            ..Config::default()
        }
    }

    fn record(sink: &Sink, bytes: &[u8]) {
        sink.submit(|r| r.file(PathBuf::from("unused"), false, |w| w.write_all(bytes)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocked_writer_bounds_admission_bytes_and_flush_without_blocking_runtime() {
        let (entered, started) = oneshot::channel();
        let entered = std::sync::Mutex::new(Some(entered));
        let (release, wait) = std::sync::mpsc::channel();
        let sink = Sink::with_writer(config(), move |_| {
            if let Some(entered) = entered.lock().unwrap().take() {
                let _ = entered.send(());
                let _ = wait.recv_timeout(Duration::from_secs(5));
            }
            Ok(())
        })
        .unwrap();
        record(&sink, b"12345678");
        tokio::time::timeout(Duration::from_secs(2), started)
            .await
            .unwrap()
            .unwrap();
        // The active record holds its byte budget while the disk is blocked.
        record(&sink, &[0; 24]);
        assert_eq!(sink.stats().pending_bytes, 32);
        record(&sink, b"x"); // byte limit, with a free queue slot
        assert_eq!(sink.stats().dropped, 1);
        sink.submit(|_| Ok(())); // occupy the remaining queue slot
        sink.submit(|_| panic!("full queue must reject before preparing a record"));
        assert_eq!(sink.stats().dropped, 2);
        assert!(!sink.flush(Duration::from_millis(20)).await);
        assert_eq!(sink.stats().pending_bytes, 32);
        release.send(()).unwrap();
        assert!(sink.flush(Duration::from_secs(2)).await);
        assert_eq!(sink.stats().pending_bytes, 0);
    }

    #[tokio::test]
    async fn oversized_or_failed_serialization_releases_all_reservations() {
        let sink = Sink::with_writer(config(), |_| {
            panic!("failed preparation must not reach disk")
        })
        .unwrap();
        sink.submit(|r| {
            r.file("unused".into(), false, |w| w.write_all(&[0; 20]))?;
            r.file("unused2".into(), false, |w| w.write_all(&[0; 5]))
        });
        sink.submit(|r| {
            r.file("unused".into(), false, |w| {
                w.write_all(b"partial")?;
                Err(io::Error::other("serialization failure"))
            })
        });
        assert_eq!(sink.stats().dropped, 2);
        assert_eq!(sink.stats().pending_bytes, 0);
        assert!(sink.flush(Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn writer_failure_is_counted_and_does_not_stop_subsequent_records() {
        let directory = tempfile::tempdir().unwrap();
        let sink = Sink::new(Config::default()).unwrap();
        sink.submit(|r| {
            r.file(directory.path().to_owned(), false, |w| {
                w.write_all(b"fails")
            })
        });
        let good = directory.path().join("ok");
        sink.submit(|r| r.file(good.clone(), false, |w| w.write_all(b"ok")));
        assert!(sink.flush(Duration::from_secs(2)).await);
        assert_eq!(sink.stats().write_failures, 1);
        assert_eq!(sink.stats().pending_bytes, 0);
        assert_eq!(fs::read(good).unwrap(), b"ok");
    }

    #[tokio::test]
    async fn concurrent_jsonl_records_are_complete_and_flush_includes_prior_records() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("events.jsonl");
        let sink = Arc::new(
            Sink::new(Config {
                queue_capacity: 1024,
                ..Config::default()
            })
            .unwrap(),
        );
        std::thread::scope(|scope| {
            for producer in 0..4 {
                let sink = &sink;
                let target = &target;
                scope.spawn(move || {
                    for event in 0..100 {
                        sink.submit(|r| {
                            r.file(target.clone(), true, |w| {
                                serde_json::to_writer(
                                    &mut *w,
                                    &serde_json::json!({"producer": producer, "event": event}),
                                )?;
                                w.write_all(b"\n")
                            })
                        });
                    }
                });
            }
        });
        assert!(sink.flush(Duration::from_secs(3)).await);
        assert_eq!(sink.stats(), Stats::default());
        let text = fs::read_to_string(target).unwrap();
        let records: std::collections::HashSet<_> = text
            .lines()
            .map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                (
                    value["producer"].as_u64().unwrap(),
                    value["event"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(records.len(), 400);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_deadline_and_timer_drop_progress_with_blocked_persistence() {
        use crate::model::Engine;
        let (entered, started) = oneshot::channel();
        let entered = std::sync::Mutex::new(Some(entered));
        let (release, wait) = std::sync::mpsc::channel();
        let sink = Sink::with_writer(Config::default(), move |_| {
            if let Some(entered) = entered.lock().unwrap().take() {
                let _ = entered.send(());
                let _ = wait.recv_timeout(Duration::from_secs(5));
            }
            Ok(())
        })
        .unwrap();
        record(&sink, b"hold writer");
        tokio::time::timeout(Duration::from_secs(2), started)
            .await
            .unwrap()
            .unwrap();
        TEST_SINK
            .scope(sink, async {
                crate::log_event!("slow_sink_test");
                let directory = tempfile::tempdir().unwrap();
                crate::benchmarking::TEST_TRACE_DIRECTORY
                    .scope(Some(directory.path().to_owned()), async {
                        let start = std::time::Instant::now();
                        crate::search::test_cancel_with_blocked_diagnostics(Engine::Bing).await;
                        assert!(start.elapsed() < Duration::from_secs(1));
                    })
                    .await;
                let sink_stats = TEST_SINK.with(Sink::stats);
                assert_eq!(sink_stats.dropped, 0);
                assert!(sink_stats.pending_bytes > 11);
                release.send(()).unwrap();
                // Drain before removing the temporary directory.
                let sender = TEST_SINK.with(|s| s.sender.clone());
                let (done, wait) = oneshot::channel();
                sender.send(Message::Flush(done)).await.unwrap();
                tokio::time::timeout(Duration::from_secs(2), wait)
                    .await
                    .unwrap()
                    .unwrap();
            })
            .await;
    }
}
