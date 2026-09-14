//! Provider parsing capacity is retained by workers, not their async callers.
use crate::error::KestrelError;
use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

/// Fixed aggregate capacity for a retained search client and all its clones.
/// Per-call search concurrency still bounds admitted provider requests; page
/// extraction's parse_concurrency is a separate resource.
#[derive(Clone)]
pub(crate) struct ParserPool(Arc<Semaphore>);

impl Default for ParserPool {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(10)))
    }
}

impl ParserPool {
    #[cfg(test)]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self(Arc::new(Semaphore::new(capacity)))
    }

    pub(crate) async fn acquire(&self) -> Result<OwnedSemaphorePermit, KestrelError> {
        self.0
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| KestrelError::Search("provider parser capacity closed".into()))
    }
}

tokio::task_local! {
    pub(crate) static POOL: ParserPool;
}

pub(crate) fn current_pool() -> ParserPool {
    // Production entry points always scope the retained client's pool. The
    // fallback also bounds internal transport/fixture entry points without one.
    static FALLBACK: std::sync::LazyLock<ParserPool> =
        std::sync::LazyLock::new(ParserPool::default);
    POOL.try_with(Clone::clone)
        .unwrap_or_else(|_| FALLBACK.clone())
}

pub(crate) async fn run<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, KestrelError> {
    let permit = current_pool().acquire().await?;
    spawn(permit, work)
        .await
        .map_err(|error| KestrelError::Search(format!("provider parser worker failed: {error}")))
}

pub(crate) fn spawn<T: Send + 'static>(
    permit: OwnedSemaphorePermit,
    work: impl FnOnce() -> T + Send + 'static,
) -> tokio::task::JoinHandle<T> {
    let context = crate::telemetry::parent_context();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _context = context.attach();
        // Consume the closure so all captured buffers and parser state are
        // destroyed before releasing capacity, on success and on unwind.
        work()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Engine, providers::response::parse_provider_response,
        search::MAX_PROVIDER_RESPONSE_BYTES,
    };
    use base64::Engine as _;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_started_work_bounds_repeated_calls_and_client_clones() {
        let mut client = crate::KestrelClient::new().unwrap();
        client.search.parsers = ParserPool(Arc::new(Semaphore::new(1)));
        let clone = client.clone();
        assert!(Arc::ptr_eq(
            &client.search.parsers.0,
            &clone.search.parsers.0
        ));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (started, ready) = tokio::sync::oneshot::channel();
        let pool = client.search.parsers.clone();
        let first = tokio::spawn(POOL.scope(
            pool,
            run(move || {
                let _ = started.send(());
                let _ = gate.recv();
            }),
        ));
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        let admitted = Arc::new(AtomicUsize::new(0));
        for _ in 0..5 {
            let admitted = admitted.clone();
            let outcome = tokio::time::timeout(
                Duration::from_millis(10),
                POOL.scope(
                    clone.search.parsers.clone(),
                    run(move || {
                        admitted.fetch_add(1, Ordering::SeqCst);
                    }),
                ),
            )
            .await;
            assert!(outcome.is_err());
            assert_eq!(client.search.parsers.0.available_permits(), 0);
        }
        assert_eq!(admitted.load(Ordering::SeqCst), 0);
        drop(release);
        tokio::time::timeout(
            Duration::from_secs(5),
            POOL.scope(client.search.parsers.clone(), run(|| 42)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(client.search.parsers.0.available_permits(), 1);
    }

    #[test]
    fn cancelled_queued_work_retains_capacity_until_buffers_are_dropped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let (started, ready) = tokio::sync::oneshot::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                let _ = gate.recv();
            });
            ready.await.unwrap();
            let pool = ParserPool(Arc::new(Semaphore::new(1)));
            struct Buffer(Arc<Semaphore>);
            impl Drop for Buffer {
                fn drop(&mut self) {
                    assert_eq!(self.0.available_permits(), 0);
                }
            }
            let buffer = Buffer(pool.0.clone());
            let job = tokio::spawn(POOL.scope(pool.clone(), run(move || drop(buffer))));
            while pool.0.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
            job.abort();
            assert!(job.await.unwrap_err().is_cancelled());
            assert!(
                tokio::time::timeout(Duration::from_millis(10), pool.acquire())
                    .await
                    .is_err()
            );
            drop(release);
            blocker.await.unwrap();
            let permit = tokio::time::timeout(Duration::from_secs(5), pool.acquire())
                .await
                .unwrap()
                .unwrap();
            drop(permit);
        });
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_html_json_and_envelope_keep_runtime_responsive() {
        let padding = "<span>large fixture</span>".repeat(10_000);
        let item = serde_json::json!({"type":"WebPage", "name":"result", "url":"https://example.org", "description":padding});
        let inner = serde_json::json!({"items":[item]}).to_string();
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&inner);
        let cases = [
            (
                Engine::Bing,
                format!(
                    "{padding}<li class='b_algo'><h2><a href='https://example.org'>result</a></h2></li>"
                ),
            ),
            (Engine::Swisscows, inner),
            (
                Engine::Swisscows,
                serde_json::json!({"payload":format!("header.{payload}.signature")}).to_string(),
            ),
        ];
        for (engine, text) in cases {
            assert!(text.len() < MAX_PROVIDER_RESPONSE_BYTES);
            let expected = parse_provider_response(engine, &text).unwrap();
            let async_thread = std::thread::current().id();
            let started = Instant::now();
            let future = run(move || {
                assert_ne!(std::thread::current().id(), async_thread);
                let parse_started = Instant::now();
                let result = parse_provider_response(engine, &text);
                (result, parse_started.elapsed())
            });
            tokio::pin!(future);
            let mut beats = 0;
            let (actual, parse_time) = loop {
                tokio::select! {
                    biased;
                    result = &mut future => break result.unwrap(),
                    _ = tokio::time::sleep(Duration::from_millis(1)) => beats += 1,
                }
            };
            assert_eq!(actual.unwrap(), expected);
            assert!(beats > 0, "{engine}: no heartbeat during large parse");
            eprintln!(
                "{engine}: CPU parse wall={parse_time:?}, worker+parse wall={:?}, heartbeat ticks={beats}; fixture allocation excluded, network=0; allocator bytes not measured",
                started.elapsed()
            );
        }
    }
}
