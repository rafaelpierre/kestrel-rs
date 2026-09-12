//! Multi-provider search, retry, normalization, and fair merging.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::{StreamExt, future::join_all, stream::FuturesUnordered};
use scraper::{ElementRef, Html, Selector};
use thiserror::Error;
use tokio::sync::Semaphore;
use url::Url;

use crate::model::{
    Engine, ProviderSearchDiagnostic, SearchMode, SearchOptions, SearchReport, SearchResult,
    SourceOccurrence, TimeFilter,
};

use crate::provider_diagnostics::{Challenge, Phase, Recorder, TransportKind};

tokio::task_local! {
    static PROVIDER_RECORDER: Arc<Mutex<Recorder>>;
    static DIAGNOSTIC_RUN_ID: String;
}

fn record_attempt() {
    let _ =
        PROVIDER_RECORDER.try_with(|state| state.lock().expect("recorder lock").start_attempt());
}

fn record_phase(phase: Phase) {
    let _ =
        PROVIDER_RECORDER.try_with(|state| state.lock().expect("recorder lock").transition(phase));
}

fn observe(update: impl FnOnce(&mut Recorder)) {
    let _ = PROVIDER_RECORDER.try_with(|state| update(&mut state.lock().expect("recorder lock")));
}

pub(crate) fn current_correlation() -> Option<serde_json::Value> {
    PROVIDER_RECORDER
        .try_with(|state| state.lock().expect("recorder lock").correlation())
        .ok()
        .flatten()
}

fn classify_challenge(engine: Engine, text: &str) -> Challenge {
    if text.trim().is_empty() {
        return Challenge::Unknown;
    }
    let document = Html::parse_document(text);
    if document.select(&selector("#b_captcha, #captcha, form[action*='captcha'], .g-recaptcha, #challenge-form, #cf-challenge-running, form[action*='anomaly.js'], .anomaly-modal")).next().is_some() {
        return Challenge::Detected;
    }
    if engine == Engine::Mojeek && crate::providers::mojeek_challenge(&document) {
        return Challenge::Detected;
    }
    if engine == Engine::Ecosia
        && document
            .select(&selector("title"))
            .any(|e| e.text().collect::<String>().contains("Firewall"))
    {
        return Challenge::Detected;
    }
    if engine == Engine::Qwant
        && serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .is_some_and(|v| v.get("url").and_then(|v| v.as_str()).is_some())
    {
        return Challenge::Detected;
    }
    // This means no known marker was detected, not proof the provider is usable.
    Challenge::NotDetected
}

fn is_tls_error(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    for _ in 0..64 {
        let Some(error) = source else { break };
        if error.is::<rustls::Error>() || error.is::<primp_tls::Error>() {
            return true;
        }
        // io::Error::source skips the wrapped error itself. Inspect get_ref()
        // first so a TLS error directly wrapped by the transport is not lost.
        source = error
            .downcast_ref::<std::io::Error>()
            .and_then(|io| {
                io.get_ref()
                    .map(|inner| inner as &(dyn std::error::Error + 'static))
            })
            .or_else(|| error.source());
    }
    false
}

fn standard_transport(error: &reqwest::Error) -> TransportKind {
    if error.is_timeout() {
        TransportKind::Timeout
    } else if is_tls_error(error) {
        TransportKind::Tls
    } else if error.is_connect() {
        TransportKind::Connect
    } else if error.is_decode() {
        TransportKind::Decode
    } else if error.is_body() {
        TransportKind::Body
    } else if error.is_request() {
        TransportKind::Request
    } else {
        TransportKind::Unknown
    }
}

fn yahoo_transport(error: &primp::Error) -> TransportKind {
    if error.is_timeout() {
        TransportKind::Timeout
    } else if is_tls_error(error) {
        TransportKind::Tls
    } else if error.is_dns() {
        TransportKind::Dns
    } else if error.is_connect() {
        TransportKind::Connect
    } else if error.is_decode() {
        TransportKind::Decode
    } else if error.is_body() {
        TransportKind::Body
    } else if error.is_request() {
        TransportKind::Request
    } else {
        TransportKind::Unknown
    }
}

const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum decompressed response bytes accepted from any search provider.
/// Independent of page-fetch limits; applies to success and HTTP error bodies.
pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum KestrelError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Search(String),
    #[error("{engine} response exceeds {limit_bytes} decoded bytes (HTTP {status})")]
    ProviderResponseTooLarge {
        engine: Engine,
        limit_bytes: usize,
        status: u16,
    },
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Yahoo request failed: {0}")]
    Yahoo(#[from] primp::Error),
    #[error("failed to initialize HTTP client: {0}")]
    Client(String),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot start blocking search from an asynchronous runtime")]
    NestedRuntime,
}

#[derive(Clone)]
pub(crate) struct SearchClients {
    pub(crate) standard: reqwest::Client,
    pub(crate) yahoo: Option<primp::Client>,
}

impl SearchClients {
    pub(crate) fn new(engines: &[Engine]) -> Result<Self, KestrelError> {
        Self::with_transport(engines, &crate::TransportOptions::default())
    }

    pub(crate) fn with_transport(
        engines: &[Engine],
        transport: &crate::TransportOptions,
    ) -> Result<Self, KestrelError> {
        transport.validate()?;
        let profile = crate::http_client::BrowserProfile::random();
        let standard = crate::http_client::standard_builder(profile, transport)
            .timeout(SEARCH_TIMEOUT)
            .build()?;
        let yahoo = engines.contains(&Engine::Yahoo).then(|| {
            let mut client = crate::http_client::impersonated_builder(profile, transport)
                .timeout(SEARCH_TIMEOUT)
                .build()?;
            *client.headers_mut() = profile.headers();
            Ok::<_, primp::Error>(client)
        });
        crate::benchmarking::capture_headers("search", &profile.headers());
        Ok(Self {
            standard,
            yahoo: yahoo.transpose()?,
        })
    }
}

/// Search one provider asynchronously.
pub async fn search(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
) -> Result<Vec<SearchResult>, KestrelError> {
    let clients = SearchClients::new(&[engine])?;
    search_with_clients(query, engine, region, time_filter, &clients).await
}

pub(crate) async fn search_with_clients(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
    clients: &SearchClients,
) -> Result<Vec<SearchResult>, KestrelError> {
    DIAGNOSTIC_RUN_ID
        .scope(uuid::Uuid::new_v4().to_string(), async {
            run_one_job(
                query,
                engine,
                Arc::new(Semaphore::new(1)),
                Arc::new(Mutex::new(Vec::new())),
                None,
                None,
                run_provider(query, engine, region, time_filter, clients),
            )
            .await
        })
        .await
}

struct ProviderResponse {
    results: Vec<SearchResult>,
    retries: usize,
    raw_result_count: usize,
}

/// Blocking compatibility wrapper for callers outside an async runtime.
pub fn search_blocking(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
) -> Result<Vec<SearchResult>, KestrelError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(KestrelError::NestedRuntime);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| KestrelError::Client(error.to_string()))?
        .block_on(search(query, engine, region, time_filter))
}

/// Search normalized unique queries according to the selected orchestration mode.
pub async fn search_many(
    queries: &[String],
    options: &SearchOptions,
) -> Result<Vec<SearchResult>, KestrelError> {
    Ok(search_many_detailed(queries, options).await?.results)
}

/// Search normalized unique queries and report each provider request's latency.
pub async fn search_many_detailed(
    queries: &[String],
    options: &SearchOptions,
) -> Result<SearchReport, KestrelError> {
    let (queries, engines) = validate_request(queries, options)?;
    let clients = SearchClients::new(&engines)?;
    search_many_with_clients_detailed(queries, engines, options, &clients).await
}

pub(crate) async fn search_many_reusing_clients(
    queries: &[String],
    options: &SearchOptions,
    clients: &SearchClients,
) -> Result<Vec<SearchResult>, KestrelError> {
    Ok(
        search_many_reusing_clients_detailed(queries, options, clients)
            .await?
            .results,
    )
}

pub(crate) async fn search_many_reusing_clients_detailed(
    queries: &[String],
    options: &SearchOptions,
    clients: &SearchClients,
) -> Result<SearchReport, KestrelError> {
    let (queries, engines) = validate_request(queries, options)?;
    search_many_with_clients_detailed(queries, engines, options, clients).await
}

async fn search_many_with_clients_detailed(
    queries: Vec<String>,
    engines: Vec<Engine>,
    options: &SearchOptions,
    clients: &SearchClients,
) -> Result<SearchReport, KestrelError> {
    DIAGNOSTIC_RUN_ID
        .scope(
            uuid::Uuid::new_v4().to_string(),
            search_many_with_clients_in_run(queries, engines, options, clients),
        )
        .await
}

async fn search_many_with_clients_in_run(
    queries: Vec<String>,
    engines: Vec<Engine>,
    options: &SearchOptions,
    clients: &SearchClients,
) -> Result<SearchReport, KestrelError> {
    let semaphore = Arc::new(Semaphore::new(options.max_concurrency));
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let deadline = options
        .search_budget
        .map(|budget| tokio::time::Instant::now() + budget);

    let (outcomes, cancelled) = match options.mode {
        SearchMode::Fanout => {
            let jobs = queries.iter().map(|query| {
                run_fanout_query(
                    query,
                    &engines,
                    clients,
                    Arc::clone(&semaphore),
                    Arc::clone(&diagnostics),
                    &options.region,
                    options.time_filter,
                    options.provider_quorum,
                    deadline,
                )
            });
            let query_outcomes = join_all(jobs).await;
            let cancelled = query_outcomes.iter().map(|(_, count)| count).sum();
            (
                query_outcomes
                    .into_iter()
                    .flat_map(|(outcomes, _)| outcomes)
                    .collect(),
                cancelled,
            )
        }
        SearchMode::Fallback => {
            let jobs = queries.iter().map(|query| {
                run_with_fallback(
                    query,
                    &engines,
                    clients,
                    Arc::clone(&semaphore),
                    Arc::clone(&diagnostics),
                    &options.region,
                    options.time_filter,
                    deadline,
                )
            });
            (join_all(jobs).await, 0)
        }
    };
    let results = merge_outcomes(outcomes, options.mode)?;
    let providers = Arc::try_unwrap(diagnostics)
        .expect("all search diagnostic references dropped")
        .into_inner()
        .expect("search diagnostic lock is not poisoned");
    Ok(SearchReport {
        results,
        providers,
        cancelled,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_fanout_query<'a>(
    query: &'a str,
    engines: &[Engine],
    clients: &'a SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    region: &'a str,
    time_filter: TimeFilter,
    provider_quorum: Option<usize>,
    deadline: Option<tokio::time::Instant>,
) -> impl Future<Output = (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)> + 'a {
    let pending = FuturesUnordered::new();
    let quorum_cancelled = Arc::new(AtomicBool::new(false));
    for (index, engine) in engines.iter().copied().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let diagnostics = Arc::clone(&diagnostics);
        let job = run_one(
            query,
            engine,
            clients,
            semaphore,
            diagnostics,
            region,
            time_filter,
            deadline,
            Some(Arc::clone(&quorum_cancelled)),
        );
        pending.push(async move { (index, job.await) });
    }
    async move { collect_fanout_signalled(pending, provider_quorum, Some(quorum_cancelled)).await }
}

#[cfg(test)]
async fn collect_fanout<F>(
    pending: FuturesUnordered<F>,
    provider_quorum: Option<usize>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    collect_fanout_signalled(pending, provider_quorum, None).await
}

async fn collect_fanout_signalled<F>(
    mut pending: FuturesUnordered<F>,
    provider_quorum: Option<usize>,
    quorum_cancelled: Option<Arc<AtomicBool>>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    let mut completed = Vec::with_capacity(pending.len());
    let mut useful = 0;
    let mut cancelled = 0;
    while let Some((index, outcome)) = pending.next().await {
        if outcome.as_ref().is_ok_and(|results| !results.is_empty()) {
            useful += 1;
        }
        completed.push((index, outcome));
        if provider_quorum.is_some_and(|quorum| useful >= quorum) {
            cancelled = pending.len();
            if let Some(signal) = &quorum_cancelled {
                signal.store(true, Ordering::Relaxed);
            }
            break;
        }
    }
    completed.sort_by_key(|(index, _)| *index);
    (
        completed.into_iter().map(|(_, outcome)| outcome).collect(),
        cancelled,
    )
}

fn validate_request(
    queries: &[String],
    options: &SearchOptions,
) -> Result<(Vec<String>, Vec<Engine>), KestrelError> {
    let mut seen_queries = HashSet::new();
    let clean_queries: Vec<String> = queries
        .iter()
        .map(|query| query.trim())
        .filter(|query| !query.is_empty())
        .filter(|query| seen_queries.insert((*query).to_owned()))
        .map(str::to_owned)
        .collect();
    let mut seen_engines = HashSet::new();
    let clean_engines: Vec<Engine> = options
        .engines
        .iter()
        .copied()
        .filter(|engine| seen_engines.insert(*engine))
        .collect();
    if clean_queries.is_empty() {
        return Err(KestrelError::InvalidRequest(
            "At least one non-empty query is required".into(),
        ));
    }
    if clean_engines.is_empty() {
        return Err(KestrelError::InvalidRequest(
            "At least one search engine is required".into(),
        ));
    }
    if options.max_concurrency < 1 {
        return Err(KestrelError::InvalidRequest(
            "max_concurrency must be at least 1".into(),
        ));
    }
    if options.search_budget.is_some_and(|budget| budget.is_zero()) {
        return Err(KestrelError::InvalidRequest(
            "search budget must be greater than zero".into(),
        ));
    }
    if let Some(quorum) = options.provider_quorum {
        if options.mode != SearchMode::Fanout {
            return Err(KestrelError::InvalidRequest(
                "provider_quorum is only valid in fanout mode".into(),
            ));
        }
        if quorum == 0 || quorum > clean_engines.len() {
            return Err(KestrelError::InvalidRequest(format!(
                "provider_quorum must be between 1 and the {} selected engines",
                clean_engines.len()
            )));
        }
    }
    Ok((clean_queries, clean_engines))
}

#[allow(clippy::too_many_arguments)]
fn run_one<'a>(
    query: &'a str,
    engine: Engine,
    clients: &'a SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    region: &'a str,
    time_filter: TimeFilter,
    deadline: Option<tokio::time::Instant>,
    quorum_cancelled: Option<Arc<AtomicBool>>,
) -> impl Future<Output = Result<Vec<SearchResult>, KestrelError>> + 'a {
    let job = run_one_job(
        query,
        engine,
        semaphore,
        diagnostics,
        deadline,
        quorum_cancelled,
        run_provider(query, engine, region, time_filter, clients),
    );
    async move {
        job.await
            .map(|results| with_provenance(results, engine, query))
    }
}

fn run_one_job<'a>(
    query: &'a str,
    engine: Engine,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    deadline: Option<tokio::time::Instant>,
    quorum_cancelled: Option<Arc<AtomicBool>>,
    provider: impl Future<Output = Result<ProviderResponse, KestrelError>> + 'a,
) -> impl Future<Output = Result<Vec<SearchResult>, KestrelError>> + 'a {
    let started = Instant::now();
    let index = {
        let mut entries = diagnostics.lock().expect("diagnostic lock");
        let index = entries.len();
        entries.push(ProviderSearchDiagnostic {
            engine,
            query: query.to_owned(),
            elapsed_ms: 0,
            result_count: 0,
            retries: 0,
            success: false,
            outcome: "cancelled_caller".into(),
            error: None,
            raw_result_count: 0,
            filtered_count: 0,
        });
        index
    };
    // Also records elapsed time when a quorum drops this future mid-request.
    let run_id = DIAGNOSTIC_RUN_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let recorder = Arc::new(Mutex::new(Recorder::with_run(run_id)));
    let timer = DiagnosticTimer {
        diagnostics: Arc::clone(&diagnostics),
        index,
        started,
        recorder: Arc::clone(&recorder),
        completed: false,
        deadline: false,
        quorum_cancelled,
    };
    async move {
        let mut timer = timer;
        recorder
            .lock()
            .expect("recorder lock")
            .transition(Phase::Queue);
        let job = PROVIDER_RECORDER.scope(Arc::clone(&recorder), async {
            let _permit = semaphore.acquire().await.expect("semaphore remains open");
            record_phase(Phase::Processing);
            provider.await
        });
        let outcome = match deadline {
            Some(deadline) if deadline <= tokio::time::Instant::now() => {
                timer.deadline = true;
                Err(KestrelError::Search("search deadline exceeded".into()))
            }
            Some(deadline) => tokio::time::timeout_at(deadline, job)
                .await
                .unwrap_or_else(|_| {
                    timer.deadline = true;
                    Err(KestrelError::Search("search deadline exceeded".into()))
                }),
            None => job.await,
        };
        timer.completed = true;
        {
            let mut entries = diagnostics.lock().expect("diagnostic lock");
            let entry = &mut entries[index];
            entry.success = outcome.is_ok();
            entry.retries = recorder.lock().expect("recorder lock").retries();
            match &outcome {
                Ok(response) => {
                    entry.result_count = response.results.len();
                    entry.raw_result_count = response.raw_result_count;
                    entry.filtered_count = response.raw_result_count - response.results.len();
                    entry.retries = response.retries;
                    entry.outcome = if response.results.is_empty() {
                        if entry.filtered_count > 0 {
                            "filtered_empty"
                        } else {
                            "empty"
                        }
                    } else {
                        "results"
                    }
                    .into();
                }
                Err(error) => {
                    let message = error.to_string();
                    entry.outcome =
                        if matches!(error, KestrelError::ProviderResponseTooLarge { .. }) {
                            "response_too_large"
                        } else {
                            provider_error_outcome(&message)
                        }
                        .into();
                    entry.error = Some(message);
                }
            }
        }
        outcome.map(|response| response.results)
    }
}

fn provider_error_outcome(message: &str) -> &'static str {
    if message.contains("deadline exceeded") {
        "deadline"
    } else if message.contains("bot challenge") {
        "challenge"
    } else if message.contains("unrecognized search page") {
        "unrecognized"
    } else {
        "request_error"
    }
}

struct DiagnosticTimer {
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    index: usize,
    started: Instant,
    recorder: Arc<Mutex<Recorder>>,
    completed: bool,
    deadline: bool,
    quorum_cancelled: Option<Arc<AtomicBool>>,
}
impl Drop for DiagnosticTimer {
    fn drop(&mut self) {
        let cancelled = !self.completed || self.deadline;
        let diagnostic = self.diagnostics.lock().ok().map(|mut entries| {
            let entry = &mut entries[self.index];
            entry.elapsed_ms = elapsed_millis(self.started);
            if !self.completed {
                entry.outcome = if self
                    .quorum_cancelled
                    .as_ref()
                    .is_some_and(|s| s.load(Ordering::Relaxed))
                {
                    "cancelled_quorum"
                } else {
                    "cancelled_caller"
                }
                .into();
            }
            if let Ok(recorder) = self.recorder.lock() {
                entry.retries = recorder.retries();
            }
            entry.clone()
        });
        if let Some(diagnostic) = diagnostic {
            let lifecycle = self
                .recorder
                .lock()
                .ok()
                .map(|mut recorder| recorder.finish_outcome(&diagnostic.outcome, cancelled));
            // Both locks are released before handing the owned snapshot to persistence.
            crate::benchmarking::capture_provider_lifecycle(&diagnostic, lifecycle.as_ref());
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_with_fallback(
    query: &str,
    engines: &[Engine],
    clients: &SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    region: &str,
    time_filter: TimeFilter,
    deadline: Option<tokio::time::Instant>,
) -> Result<Vec<SearchResult>, KestrelError> {
    collect_fallback(query, engines, |engine| {
        run_one(
            query,
            engine,
            clients,
            Arc::clone(&semaphore),
            Arc::clone(&diagnostics),
            region,
            time_filter,
            deadline,
            None,
        )
    })
    .await
}

async fn collect_fallback<F, Fut>(
    query: &str,
    engines: &[Engine],
    mut run: F,
) -> Result<Vec<SearchResult>, KestrelError>
where
    F: FnMut(Engine) -> Fut,
    Fut: Future<Output = Result<Vec<SearchResult>, KestrelError>>,
{
    let mut errors = Vec::new();
    let mut had_empty = false;
    for engine in engines {
        match run(*engine).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => {
                had_empty = true;
            }
            Err(error) => {
                crate::log_event!("search_fallback", "query" => query, "failed_engine" => engine.as_str(), "error" => error.to_string());
                errors.push(error.to_string());
            }
        }
    }
    if had_empty {
        return Ok(Vec::new());
    }
    Err(KestrelError::Search(format!(
        "All engines failed for query {query:?}: {}",
        errors.join("; ")
    )))
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

async fn run_provider(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
    clients: &SearchClients,
) -> Result<ProviderResponse, KestrelError> {
    let mut result = match engine {
        Engine::Duckduckgo => {
            search_duckduckgo(query, region, time_filter, &clients.standard).await
        }
        Engine::Bing => search_bing(query, region, time_filter, &clients.standard).await,
        Engine::Yahoo => {
            search_yahoo(
                query,
                region,
                time_filter,
                clients.yahoo.as_ref().expect("Yahoo client initialized"),
            )
            .await
        }
        _ => search_additional(query, engine, region, time_filter, &clients.standard).await,
    };
    record_phase(Phase::Processing);
    if let Ok(response) = &mut result {
        response.raw_result_count = response.results.len();
        for (index, result) in response.results.iter_mut().enumerate() {
            result.engine_rank = Some(index + 1);
        }
        response
            .results
            .retain(|result| result_allowed(query, &result.url));
    }
    if let Err(error) = &result {
        crate::log_event!(
            "search_failed",
            "engine" => engine.as_str(),
            "query" => query,
            "error_type" => "request",
            "error" => error.to_string(),
        );
    } else if result
        .as_ref()
        .is_ok_and(|response| response.results.is_empty())
    {
        crate::log_event!("search_no_results", "engine" => engine.as_str(), "query" => query);
    }
    result
}

async fn search_additional(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
    client: &reqwest::Client,
) -> Result<ProviderResponse, KestrelError> {
    // Validate before entering retry machinery; builders below cannot fail validation.
    let _ = crate::providers::request(client, engine, query, region, time_filter)?;
    let (text, retries) = request_standard_with_retries(client, engine, query, || {
        crate::providers::request(client, engine, query, region, time_filter)
            .expect("validated provider request")
    })
    .await?;
    Ok(ProviderResponse {
        results: crate::providers::parse(engine, &text)?,
        retries,
        raw_result_count: 0,
    })
}

async fn search_duckduckgo(
    query: &str,
    region: &str,
    time_filter: TimeFilter,
    client: &reqwest::Client,
) -> Result<ProviderResponse, KestrelError> {
    let mut data = vec![("q", query)];
    if !region.is_empty() {
        data.push(("kl", region));
    }
    if time_filter != TimeFilter::Any {
        data.push(("df", time_filter.as_str()));
    }
    let (text, retries) = request_standard_with_retries(client, Engine::Duckduckgo, query, || {
        client.post("https://html.duckduckgo.com/html/").form(&data)
    })
    .await?;
    Ok(ProviderResponse {
        results: parse_duckduckgo_response(&text)?,
        retries,
        raw_result_count: 0,
    })
}

async fn search_bing(
    query: &str,
    region: &str,
    time_filter: TimeFilter,
    client: &reqwest::Client,
) -> Result<ProviderResponse, KestrelError> {
    let mut params = vec![("q", query)];
    let country = region
        .split_once('-')
        .map_or(region, |(country, _)| country);
    if !country.is_empty() {
        params.push(("cc", country));
    }
    if time_filter != TimeFilter::Any {
        crate::log_event!(
            "search_filter_unsupported",
            "engine" => "bing",
            "query" => query,
            "filter" => "time_filter",
            "value" => time_filter.as_str(),
        );
    }
    let (text, retries) = request_standard_with_retries(client, Engine::Bing, query, || {
        client.get("https://www.bing.com/search").query(&params)
    })
    .await?;
    Ok(ProviderResponse {
        results: parse_provider_response(Engine::Bing, &text)?,
        retries,
        raw_result_count: 0,
    })
}

async fn search_yahoo(
    query: &str,
    region: &str,
    time_filter: TimeFilter,
    client: &primp::Client,
) -> Result<ProviderResponse, KestrelError> {
    let mut params = vec![("p", query), ("ei", "UTF-8")];
    if !region.is_empty() {
        params.push(("vl", region));
    }
    if time_filter != TimeFilter::Any {
        params.push(("btf", time_filter.as_str()));
    }
    let (html, retries) = request_yahoo_with_retries(query, || {
        client
            .get("https://search.yahoo.com/search")
            .query(&params)
            .timeout(SEARCH_TIMEOUT)
    })
    .await?;
    Ok(ProviderResponse {
        results: parse_provider_response(Engine::Yahoo, &html)?,
        retries,
        raw_result_count: 0,
    })
}

async fn request_yahoo_with_retries<F>(
    query: &str,
    build: F,
) -> Result<(String, usize), KestrelError>
where
    F: Fn() -> primp::RequestBuilder,
{
    let engine = Engine::Yahoo;
    let mut last_error = None;
    for attempt in 1..=3 {
        record_attempt();
        let response = build().send().await;
        observe(|r| {
            r.transition_censored(
                Phase::Processing,
                response.as_ref().err().is_some_and(|e| e.is_timeout()),
            )
        });
        match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                observe(|r| r.headers(status, retry_after));
                let final_url = response.url().to_string();
                let http_version = format!("{:?}", response.version());
                record_phase(Phase::Body);
                let body = read_yahoo_body(response).await;
                observe(|r| {
                    r.transition_censored(
                        Phase::Processing,
                        body.as_ref().err().is_some_and(body_read_censored),
                    )
                });
                match body {
                    Ok(html) => {
                        record_phase(Phase::Parse);
                        let challenge = classify_challenge(engine, &html);
                        observe(|r| r.response(challenge));
                        record_phase(Phase::Processing);
                        crate::benchmarking::capture_provider(
                            engine,
                            query,
                            &final_url,
                            status,
                            &http_version,
                            attempt,
                            &html,
                        );
                        if (200..300).contains(&status) {
                            record_phase(Phase::Parse);
                            return Ok((html, attempt - 1));
                        }
                    }
                    Err(error) => {
                        record_body_error(&error);
                        // Preserve the successful-status body failure policy. For error
                        // statuses retain status-based retries, while keeping the body error.
                        if (200..300).contains(&status)
                            || matches!(error, KestrelError::ProviderResponseTooLarge { .. })
                        {
                            return Err(error);
                        }
                    }
                }
                let retryable = status == 408 || status == 429 || status >= 500;
                let error = KestrelError::Search(format!("{engine} returned HTTP {status}"));
                if !retryable || attempt == 3 {
                    return Err(error);
                }
                last_error = Some(error);
            }
            Err(error) => {
                observe(|r| r.error(yahoo_transport(&error), false));
                if attempt == 3 {
                    return Err(error.into());
                }
                last_error = Some(error.into());
            }
        }
        log_retry(engine, query, attempt, last_error.as_ref());
        retry_delay(attempt).await;
    }
    Err(last_error.unwrap_or_else(|| KestrelError::Search("request failed".into())))
}
async fn request_standard_with_retries<F>(
    _client: &reqwest::Client,
    engine: Engine,
    query: &str,
    build: F,
) -> Result<(String, usize), KestrelError>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut last_error = None;
    for attempt in 1..=3 {
        record_attempt();
        let response = build().send().await;
        observe(|r| {
            r.transition_censored(
                Phase::Processing,
                response.as_ref().err().is_some_and(|e| e.is_timeout()),
            )
        });
        match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                observe(|r| r.headers(status, retry_after));
                let final_url = response.url().to_string();
                let http_version = format!("{:?}", response.version());
                record_phase(Phase::Body);
                let body = read_standard_body(response, engine).await;
                observe(|r| {
                    r.transition_censored(
                        Phase::Processing,
                        body.as_ref().err().is_some_and(body_read_censored),
                    )
                });
                match body {
                    Ok(html) => {
                        record_phase(Phase::Parse);
                        let challenge = classify_challenge(engine, &html);
                        observe(|r| r.response(challenge));
                        record_phase(Phase::Processing);
                        crate::benchmarking::capture_provider(
                            engine,
                            query,
                            &final_url,
                            status,
                            &http_version,
                            attempt,
                            &html,
                        );
                        if (200..300).contains(&status) {
                            record_phase(Phase::Parse);
                            return Ok((html, attempt - 1));
                        }
                    }
                    Err(error) => {
                        record_body_error(&error);
                        // Preserve the successful-status body failure policy. For error
                        // statuses retain status-based retries, while keeping the body error.
                        if (200..300).contains(&status)
                            || matches!(error, KestrelError::ProviderResponseTooLarge { .. })
                        {
                            return Err(error);
                        }
                    }
                }
                let retryable = status == 408 || status == 429 || status >= 500;
                let error = KestrelError::Search(format!("{engine} returned HTTP {status}"));
                if !retryable || attempt == 3 {
                    return Err(error);
                }
                last_error = Some(error);
            }
            Err(error) => {
                observe(|r| r.error(standard_transport(&error), false));
                let retryable = error.is_timeout() || error.is_connect() || error.is_request();
                if !retryable || attempt == 3 {
                    return Err(error.into());
                }
                last_error = Some(error.into());
            }
        }
        log_retry(engine, query, attempt, last_error.as_ref());
        retry_delay(attempt).await;
    }
    Err(last_error.unwrap_or_else(|| KestrelError::Search("request failed".into())))
}

fn body_read_censored(error: &KestrelError) -> bool {
    match error {
        KestrelError::Http(error) => error.is_timeout(),
        KestrelError::Yahoo(error) => error.is_timeout(),
        KestrelError::ProviderResponseTooLarge { .. } => true,
        _ => false,
    }
}

fn record_body_error(error: &KestrelError) {
    observe(|recorder| match error {
        KestrelError::Http(error) => recorder.error(standard_transport(error), true),
        KestrelError::Yahoo(error) => recorder.error(yahoo_transport(error), true),
        KestrelError::ProviderResponseTooLarge { .. } => recorder.response_too_large(),
        _ => recorder.error(TransportKind::Unknown, true),
    });
}

// Both transports expose decompressed chunks. Check before appending, including
// when Content-Length is absent (chunked transfer or automatic decompression).
async fn read_standard_body(
    mut response: reqwest::Response,
    engine: Engine,
) -> Result<String, KestrelError> {
    let mut body = ProviderBody::new(
        engine,
        response.status().as_u16(),
        response.content_length(),
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
    )?;
    while let Some(chunk) = response.chunk().await? {
        body.push(&chunk)?;
    }
    Ok(body.text())
}

async fn read_yahoo_body(mut response: primp::Response) -> Result<String, KestrelError> {
    let mut body = ProviderBody::new(
        Engine::Yahoo,
        response.status().as_u16(),
        response.content_length(),
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
    )?;
    while let Some(chunk) = response.chunk().await? {
        body.push(&chunk)?;
    }
    Ok(body.text())
}

struct ProviderBody {
    bytes: Vec<u8>,
    encoding: &'static encoding_rs::Encoding,
    engine: Engine,
    status: u16,
}

impl ProviderBody {
    fn new(
        engine: Engine,
        status: u16,
        content_length: Option<u64>,
        content_type: Option<&str>,
    ) -> Result<Self, KestrelError> {
        let mime = content_type.and_then(|value| value.parse::<mime::Mime>().ok());
        let encoding = mime
            .as_ref()
            .and_then(|mime| mime.get_param("charset"))
            .and_then(|charset| encoding_rs::Encoding::for_label(charset.as_str().as_bytes()))
            .unwrap_or(encoding_rs::UTF_8);
        let body = Self {
            bytes: Vec::new(),
            encoding,
            engine,
            status,
        };
        if content_length.is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64) {
            return Err(body.too_large());
        }
        Ok(body)
    }

    fn too_large(&self) -> KestrelError {
        KestrelError::ProviderResponseTooLarge {
            engine: self.engine,
            limit_bytes: MAX_PROVIDER_RESPONSE_BYTES,
            status: self.status,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), KestrelError> {
        if chunk.len() > MAX_PROVIDER_RESPONSE_BYTES - self.bytes.len() {
            return Err(self.too_large());
        }
        let required = self.bytes.len() + chunk.len();
        if required > self.bytes.capacity() {
            // Retain amortized growth without asking Vec to grow past the cap.
            let capacity = required
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_PROVIDER_RESPONSE_BYTES);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn text(self) -> String {
        // Match Response::text's charset/BOM handling and replacement semantics.
        // UTF-8 expansion and parser allocations remain proportional to the cap.
        self.encoding.decode(&self.bytes).0.into_owned()
    }
}

fn log_retry(engine: Engine, query: &str, attempt: usize, error: Option<&KestrelError>) {
    crate::log_event!(
        "search_retry",
        "engine" => engine.as_str(),
        "query" => query,
        "attempt" => attempt,
        "error_type" => "request",
        "error" => error.map(ToString::to_string).unwrap_or_default(),
    );
}

#[cfg(test)]
tokio::task_local! {
    static TEST_RETRY_DELAY: Duration;
    static TEST_BACKOFF_ENTERED: Arc<tokio::sync::Notify>;
}

async fn retry_delay(attempt: usize) {
    record_phase(Phase::Backoff);
    #[cfg(test)]
    let _ = TEST_BACKOFF_ENTERED.try_with(|notify| notify.notify_one());
    #[cfg(test)]
    if let Ok(delay) = TEST_RETRY_DELAY.try_with(|delay| *delay) {
        tokio::time::sleep(delay).await;
        return;
    }
    #[cfg(test)]
    if attempt > 0 {
        return;
    }
    let base_ms = (250_u64 * 2_u64.pow((attempt.saturating_sub(1)) as u32)).min(2_000);
    let jitter_ms = (rand::random::<f64>() * base_ms as f64) as u64;
    tokio::time::sleep(Duration::from_millis((base_ms + jitter_ms).min(2_000))).await;
}

fn with_provenance(results: Vec<SearchResult>, engine: Engine, query: &str) -> Vec<SearchResult> {
    results
        .into_iter()
        .enumerate()
        .map(|(index, mut result)| {
            let rank = result.engine_rank.unwrap_or(index + 1);
            result.engine = Some(engine);
            result.query = Some(query.to_owned());
            result.engine_rank = Some(rank);
            result.sources = vec![SourceOccurrence {
                engine,
                query: query.to_owned(),
                rank,
            }];
            result
        })
        .collect()
}

fn merge_outcomes(
    outcomes: Vec<Result<Vec<SearchResult>, KestrelError>>,
    mode: SearchMode,
) -> Result<Vec<SearchResult>, KestrelError> {
    let mut buckets = Vec::new();
    let mut failures = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(results) => buckets.push(results),
            Err(error) => failures.push(error),
        }
    }
    if buckets.is_empty() && !failures.is_empty() {
        return Err(KestrelError::Search(format!(
            "Every search failed: {}",
            failures
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    if !failures.is_empty() {
        crate::log_event!(
            "search_partial_failure",
            "mode" => mode.to_string(),
            "failure_count" => failures.len(),
            "success_count" => buckets.len(),
        );
    }
    Ok(merge_round_robin(buckets))
}

fn merge_round_robin(buckets: Vec<Vec<SearchResult>>) -> Vec<SearchResult> {
    let mut iterators: Vec<_> = buckets.into_iter().map(Vec::into_iter).collect();
    let mut merged = Vec::new();
    let mut by_url = HashMap::new();
    loop {
        let mut advanced = false;
        for iterator in &mut iterators {
            let Some(item) = iterator.next() else {
                continue;
            };
            advanced = true;
            let key = result_key(&item);
            if let Some(existing_index) = by_url.get(&key).copied() {
                let existing: &mut SearchResult = &mut merged[existing_index];
                existing.sources.extend(item.sources);
            } else {
                by_url.insert(key, merged.len());
                merged.push(item);
            }
        }
        if !advanced {
            break;
        }
    }
    merged
}

fn result_key(result: &SearchResult) -> String {
    let canonical = canonical_url(&result.url);
    if canonical.is_empty() {
        format!("{}\0{}", result.title, result.snippet)
    } else {
        canonical
    }
}

pub(crate) fn canonical_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return String::new();
    };
    url.set_fragment(None);
    let retained: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_") && !matches!(key.as_str(), "fbclid" | "gclid" | "msclkid")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !retained.is_empty() {
        url.query_pairs_mut().extend_pairs(retained);
    }
    let trimmed_path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if trimmed_path.is_empty() {
        "/"
    } else {
        &trimmed_path
    });
    url.to_string()
}

/// Split at whitespace outside quoted phrases, preserving all query characters.
pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    for c in query.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

/// Extract only unambiguous, standalone positive hostname restrictions.
/// Compound boolean expressions, quoted operators and path filters remain upstream.
pub(crate) fn site_domain(query: &str) -> Option<String> {
    let terms = query_tokens(query);
    if query.contains(['(', ')', '|'])
        || terms
            .iter()
            .any(|t| t.eq_ignore_ascii_case("OR") || t.eq_ignore_ascii_case("NOT"))
    {
        return None;
    }
    let sites: Vec<_> = terms
        .iter()
        .filter_map(|token| {
            token
                .get(..5)
                .filter(|prefix| prefix.eq_ignore_ascii_case("site:"))
                .map(|_| &token[5..])
        })
        .collect();
    if sites.len() != 1 {
        return None;
    }
    let domain = sites[0].trim_end_matches('.').to_ascii_lowercase();
    if !domain.contains('.')
        || !domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return None;
    }
    Some(domain)
}

fn result_allowed(query: &str, value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.');
    site_domain(query).is_none_or(|domain| host == domain || host.ends_with(&format!(".{domain}")))
}

fn parse_provider_response(engine: Engine, html: &str) -> Result<Vec<SearchResult>, KestrelError> {
    let document = Html::parse_document(html);
    if document
        .select(&selector(
            "#b_captcha, #captcha, form[action*='captcha'], .g-recaptcha",
        ))
        .next()
        .is_some()
    {
        return Err(KestrelError::Search(format!(
            "{engine} returned a bot challenge"
        )));
    }
    let results = match engine {
        Engine::Bing => parse_bing_results(html),
        Engine::Yahoo => parse_yahoo_results(html),
        Engine::Duckduckgo => return parse_duckduckgo_response(html),
        _ => return crate::providers::parse(engine, html),
    };
    let empty_marker = match engine {
        Engine::Bing => "li.b_no, .b_no",
        Engine::Yahoo => ".msgNoResults, .no-results",
        _ => unreachable!(),
    };
    if results.is_empty() && document.select(&selector(empty_marker)).next().is_none() {
        return Err(KestrelError::Search(format!(
            "{engine} returned an unrecognized search page"
        )));
    }
    Ok(results)
}

fn parse_duckduckgo_response(html: &str) -> Result<Vec<SearchResult>, KestrelError> {
    let document = Html::parse_document(html);
    if document
        .select(&selector(
            "form#challenge-form, form[action*='anomaly.js'], .anomaly-modal",
        ))
        .next()
        .is_some()
    {
        return Err(KestrelError::Search(
            "DuckDuckGo returned a bot challenge; try --engine bing or --engine yahoo".into(),
        ));
    }
    let results = parse_duckduckgo_results(html);
    if results.is_empty() && document.select(&selector(".no-results")).next().is_none() {
        return Err(KestrelError::Search(
            "DuckDuckGo returned an unrecognized search page; try --engine bing or --engine yahoo"
                .into(),
        ));
    }
    Ok(results)
}

fn parse_duckduckgo_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let item = selector("div.result.results_links.results_links_deep.web-result");
    let title = selector("h2.result__title a.result__a");
    let display = selector("a.result__url");
    let snippet = selector("a.result__snippet");
    document
        .select(&item)
        .filter_map(|entry| {
            let link = entry.select(&title).next()?;
            Some(SearchResult::parsed(
                element_text(link, ""),
                link.value().attr("href").unwrap_or_default().to_owned(),
                entry
                    .select(&display)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, "")),
                entry
                    .select(&snippet)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, "")),
            ))
        })
        .collect()
}

fn parse_bing_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let item = selector("li.b_algo");
    let title = selector("h2 a");
    let snippet = selector(".b_caption p");
    let display = selector(".b_attribution cite, cite");
    document
        .select(&item)
        .filter_map(|entry| {
            let link = entry.select(&title).next()?;
            Some(SearchResult::parsed(
                element_text(link, " "),
                decode_bing_url(link.value().attr("href").unwrap_or_default()),
                entry
                    .select(&display)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, " ")),
                entry
                    .select(&snippet)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, " ")),
            ))
        })
        .collect()
}

fn decode_bing_url(value: &str) -> String {
    if !value.contains("/ck/a") && !value.contains("/cr?") {
        return value.to_owned();
    }
    let Ok(url) = Url::parse(value) else {
        return value.to_owned();
    };
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    if let Some(target) = params.get("rurl") {
        return target.to_owned();
    }
    let Some(encoded) = params.get("u") else {
        return value.to_owned();
    };
    let encoded = encoded.strip_prefix("a1").unwrap_or(encoded);
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded))
        .ok()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .unwrap_or_else(|| value.to_owned())
}

fn parse_yahoo_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let primary = selector("div.dd.algo");
    let fallback = selector("div.compTitle");
    let entries: Vec<_> = document.select(&primary).collect();
    let entries = if entries.is_empty() {
        document.select(&fallback).collect()
    } else {
        entries
    };
    let title_link = selector(".compTitle > a, h3 a");
    let title_selector = selector("h3");
    let snippet = selector(".compText p, .compText");
    entries
        .into_iter()
        .filter_map(|entry| {
            let is_title_only = entry.value().classes().any(|class| class == "compTitle");
            let container = if is_title_only {
                yahoo_result_container(entry, &snippet)
            } else {
                entry
            };
            let link = entry
                .select(&title_link)
                .next()
                .or_else(|| container.select(&title_link).next())?;
            let raw_url = link.value().attr("href").unwrap_or_default();
            let url = decode_yahoo_url(raw_url);
            let title = link
                .value()
                .attr("aria-label")
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    link.select(&title_selector)
                        .next()
                        .or_else(|| entry.select(&title_selector).next())
                        .or_else(|| container.select(&title_selector).next())
                        .map_or_else(|| element_text(link, " "), |value| element_text(value, " "))
                });
            let display_url = Url::parse(&url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
                .unwrap_or_default();
            Some(SearchResult::parsed(
                title,
                url,
                display_url,
                container
                    .select(&snippet)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, " ")),
            ))
        })
        .collect()
}

fn yahoo_result_container<'a>(entry: ElementRef<'a>, snippet: &Selector) -> ElementRef<'a> {
    let mut container = entry;
    for _ in 0..5 {
        let Some(parent) = container.parent().and_then(ElementRef::wrap) else {
            break;
        };
        container = parent;
        if container.select(snippet).next().is_some() {
            break;
        }
    }
    container
}

fn decode_yahoo_url(value: &str) -> String {
    let Some(encoded) = value
        .split("/RU=")
        .nth(1)
        .and_then(|tail| tail.split("/RK=").next())
    else {
        return value.to_owned();
    };
    url::form_urlencoded::parse(encoded.as_bytes())
        .next()
        .map(|(decoded, _)| decoded.into_owned())
        .unwrap_or_else(|| percent_decode(encoded))
}

fn percent_decode(value: &str) -> String {
    let with_prefix = format!("x={value}");
    url::form_urlencoded::parse(with_prefix.as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| value.to_owned())
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static selector is valid")
}

fn element_text(element: ElementRef<'_>, separator: &str) -> String {
    element
        .text()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const DDG: &str = r#"
    <div class="result results_links results_links_deep web-result">
      <h2 class="result__title"><a class="result__a" href="https://example.com">Example</a></h2>
      <a class="result__url">example.com</a><a class="result__snippet">A useful result</a>
    </div><div class="result results_links results_links_deep web-result"><h2></h2></div>"#;

    fn assert_oversized(error: KestrelError, engine: Engine, expected_status: u16) {
        assert!(
            matches!(
                error,
                KestrelError::ProviderResponseTooLarge { engine: actual_engine, limit_bytes, status }
                    if actual_engine == engine && limit_bytes == MAX_PROVIDER_RESPONSE_BYTES
                        && status == expected_status
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn provider_buffer_checks_capacity_before_appending() {
        let mut body = ProviderBody::new(Engine::Bing, 200, None, None).unwrap();
        // Uneven chunks exercise growth near the limit rather than powers of two.
        let chunk = vec![b'x'; 100_003];
        while body.bytes.len() + chunk.len() <= MAX_PROVIDER_RESPONSE_BYTES {
            body.push(&chunk).unwrap();
            assert!(body.bytes.capacity() <= MAX_PROVIDER_RESPONSE_BYTES);
        }
        body.push(&vec![b'x'; MAX_PROVIDER_RESPONSE_BYTES - body.bytes.len()])
            .unwrap();
        let capacity = body.bytes.capacity();
        assert_oversized(body.push(b"x").unwrap_err(), Engine::Bing, 200);
        assert_eq!(body.bytes.len(), MAX_PROVIDER_RESPONSE_BYTES);
        assert_eq!(body.bytes.capacity(), capacity);
        assert!(capacity <= MAX_PROVIDER_RESPONSE_BYTES);
        assert_eq!(body.text().len(), MAX_PROVIDER_RESPONSE_BYTES);
    }

    async fn raw_provider_request(
        yahoo: bool,
        response: Vec<u8>,
        hold_open: bool,
    ) -> Result<(String, usize), KestrelError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "client closed before sending request headers");
                request.extend_from_slice(&chunk[..read]);
                assert!(request.len() <= 16_384, "unexpectedly large test request");
            }
            let _ = socket.write_all(&response).await;
            if hold_open {
                std::future::pending::<()>().await;
            }
        });
        let attempts = Arc::new(Mutex::new(Recorder::new()));
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            PROVIDER_RECORDER.scope(Arc::clone(&attempts), async {
                if yahoo {
                    let client = primp::Client::builder().no_proxy().build().unwrap();
                    request_yahoo_with_retries("test", || client.get(&url)).await
                } else {
                    let client = reqwest::Client::builder().no_proxy().build().unwrap();
                    request_standard_with_retries(&client, Engine::Bing, "test", || {
                        client.get(&url)
                    })
                    .await
                }
            }),
        )
        .await;
        server.abort();
        assert_eq!(
            attempts.lock().unwrap().finish(false).send_attempts,
            1,
            "oversize must not retry"
        );
        result.expect("reader must not wait for an oversized declared body")
    }

    #[tokio::test]
    async fn provider_transports_bound_declared_chunked_and_compressed_bodies() {
        use std::io::Write;
        let oversized = vec![b'x'; MAX_PROVIDER_RESPONSE_BYTES + 1];
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&oversized).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(compressed.len() < MAX_PROVIDER_RESPONSE_BYTES);
        for yahoo in [false, true] {
            let engine = if yahoo { Engine::Yahoo } else { Engine::Bing };
            for status in [200, 403, 429, 500] {
                // No body is sent: the Content-Length check must reject immediately.
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n\r\n",
                    oversized.len()
                );
                assert_oversized(
                    raw_provider_request(yahoo, response.into_bytes(), true)
                        .await
                        .unwrap_err(),
                    engine,
                    status,
                );

                let mut response =
                    format!("HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\n\r\n")
                        .into_bytes();
                for chunk in oversized.chunks(16_384) {
                    response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                    response.extend_from_slice(chunk);
                    response.extend_from_slice(b"\r\n");
                }
                response.extend_from_slice(b"0\r\n\r\n");
                assert_oversized(
                    raw_provider_request(yahoo, response, false)
                        .await
                        .unwrap_err(),
                    engine,
                    status,
                );

                let mut response = format!("HTTP/1.1 {status} Test\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n", compressed.len()).into_bytes();
                response.extend_from_slice(&compressed);
                assert_oversized(
                    raw_provider_request(yahoo, response, false)
                        .await
                        .unwrap_err(),
                    engine,
                    status,
                );
            }
        }
    }

    #[tokio::test]
    async fn provider_transports_accept_boundary_and_preserve_text_decoding() {
        use std::io::Write;
        for yahoo in [false, true] {
            for bytes in [
                Vec::new(),
                vec![b'x'; MAX_PROVIDER_RESPONSE_BYTES],
                vec![b'c', b'a', b'f', 0xe9],
            ] {
                let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=windows-1252\r\nContent-Length: {}\r\n\r\n", bytes.len()).into_bytes();
                response.extend_from_slice(&bytes);
                let (text, retries) = raw_provider_request(yahoo, response, false).await.unwrap();
                assert_eq!(text, encoding_rs::WINDOWS_1252.decode(&bytes).0);
                assert_eq!(retries, 0);
            }
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(DDG.as_bytes()).unwrap();
            let compressed = encoder.finish().unwrap();
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
                compressed.len()
            )
            .into_bytes();
            response.extend_from_slice(&compressed);
            let (html, _) = raw_provider_request(yahoo, response, false).await.unwrap();
            assert_eq!(html, DDG);
            assert_eq!(parse_duckduckgo_response(&html).unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn provider_http_errors_keep_retry_policy_under_limit() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        for yahoo in [false, true] {
            for status in [403, 503] {
                let server = MockServer::start().await;
                let calls = Arc::new(AtomicUsize::new(0));
                let count = Arc::clone(&calls);
                Mock::given(method("GET"))
                    .respond_with(move |_: &wiremock::Request| {
                        let call = count.fetch_add(1, Ordering::Relaxed);
                        if call == 0 {
                            ResponseTemplate::new(status).set_body_string("unavailable")
                        } else {
                            ResponseTemplate::new(200).set_body_string(DDG)
                        }
                    })
                    .mount(&server)
                    .await;
                let result = if yahoo {
                    let client = primp::Client::builder().no_proxy().build().unwrap();
                    request_yahoo_with_retries("test", || client.get(server.uri())).await
                } else {
                    let client = reqwest::Client::builder().no_proxy().build().unwrap();
                    request_standard_with_retries(&client, Engine::Bing, "test", || {
                        client.get(server.uri())
                    })
                    .await
                };
                if status == 403 {
                    assert!(result.unwrap_err().to_string().contains("HTTP 403"));
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                } else {
                    let (text, retries) = result.unwrap();
                    assert_eq!(text, DDG);
                    assert_eq!(retries, 1);
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                }
            }
        }
    }

    #[tokio::test]
    async fn provider_body_failures_preserve_status_retries() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for yahoo in [false, true] {
            for status in [200, 403, 408, 429, 503] {
                for failure in ["truncated", "gzip", "timeout", "exhausted"] {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let url = format!("http://{}/", listener.local_addr().unwrap());
                    let calls = Arc::new(AtomicUsize::new(0));
                    let count = Arc::clone(&calls);
                    let server = tokio::spawn(async move {
                        let mut connections = tokio::task::JoinSet::new();
                        loop {
                            let (mut socket, _) = listener.accept().await.unwrap();
                            let count = Arc::clone(&count);
                            connections.spawn(async move {
                                let mut request = Vec::new();
                                let mut chunk = [0; 1024];
                                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                                    let read = socket.read(&mut chunk).await.unwrap();
                                    assert!(read > 0);
                                    request.extend_from_slice(&chunk[..read]);
                                }
                                let call = count.fetch_add(1, Ordering::Relaxed);
                                let response = if call > 0 && failure != "exhausted" {
                                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
                                } else if failure == "gzip" {
                                    format!("HTTP/1.1 {status} Test\r\nContent-Encoding: gzip\r\nContent-Length: 4\r\nConnection: close\r\n\r\noops")
                                } else {
                                    format!("HTTP/1.1 {status} Test\r\nContent-Length: 100\r\nConnection: close\r\n\r\nx")
                                };
                                socket.write_all(response.as_bytes()).await.unwrap();
                                if failure == "timeout" && call == 0 {
                                    std::future::pending::<()>().await;
                                }
                            });
                        }
                    });
                    let result = tokio::time::timeout(Duration::from_secs(5), async {
                        if yahoo {
                            let client = primp::Client::builder().no_proxy().build().unwrap();
                            request_yahoo_with_retries("test", || {
                                client.get(&url).timeout(Duration::from_millis(300))
                            })
                            .await
                        } else {
                            let client = reqwest::Client::builder().no_proxy().build().unwrap();
                            request_standard_with_retries(&client, Engine::Bing, "test", || {
                                client.get(&url).timeout(Duration::from_millis(300))
                            })
                            .await
                        }
                    })
                    .await;
                    server.abort();
                    let result = result.expect("retry sequence must finish");
                    let retryable = matches!(status, 408 | 429 | 503);
                    let expected_calls = if !retryable {
                        1
                    } else if failure == "exhausted" {
                        3
                    } else {
                        2
                    };
                    assert_eq!(
                        calls.load(Ordering::Relaxed),
                        expected_calls,
                        "yahoo={yahoo}, status={status}, failure={failure}"
                    );
                    if retryable && failure != "exhausted" {
                        assert_eq!(result.unwrap(), ("ok".into(), 1));
                    } else if status == 200 {
                        assert!(matches!(
                            result.unwrap_err(),
                            KestrelError::Http(_) | KestrelError::Yahoo(_)
                        ));
                    } else {
                        assert!(
                            result
                                .unwrap_err()
                                .to_string()
                                .contains(&format!("HTTP {status}"))
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn oversized_provider_preserves_fallback_and_partial_success() {
        let oversized = || KestrelError::ProviderResponseTooLarge {
            engine: Engine::Bing,
            limit_bytes: MAX_PROVIDER_RESPONSE_BYTES,
            status: 200,
        };
        let mut calls = Vec::new();
        let results = collect_fallback("query", &[Engine::Bing, Engine::Duckduckgo], |engine| {
            calls.push(engine);
            std::future::ready(if engine == Engine::Bing {
                Err(oversized())
            } else {
                Ok(parse_duckduckgo_response(DDG).unwrap())
            })
        })
        .await
        .unwrap();
        assert_eq!(calls, [Engine::Bing, Engine::Duckduckgo]);
        assert_eq!(results.len(), 1);
        let merged =
            merge_outcomes(vec![Err(oversized()), Ok(results)], SearchMode::Fanout).unwrap();
        assert_eq!(merged.len(), 1);
        let error = merge_outcomes(vec![Err(oversized())], SearchMode::Fanout)
            .unwrap_err()
            .to_string();
        assert!(error.contains("bing response exceeds 4194304 decoded bytes (HTTP 200)"));
    }

    #[test]
    fn parses_duckduckgo() {
        let results = parse_duckduckgo_response(DDG).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example");
        assert_eq!(results[0].snippet, "A useful result");
    }

    #[test]
    fn duckduckgo_distinguishes_empty_results_from_unrecognized_pages() {
        assert!(
            parse_duckduckgo_response(r#"<div class="no-results">No results found.</div>"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_duckduckgo_response("<html><body>Service unavailable</body></html>")
                .unwrap_err()
                .to_string()
                .contains("unrecognized search page")
        );
    }

    #[tokio::test]
    async fn duckduckgo_challenge_is_an_error_even_with_success_http_status() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

        for status in [200, 202] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_string(
                    r#"<form id="challenge-form" action="//duckduckgo.com/anomaly.js"><div class="anomaly-modal">Please confirm you are human</div></form>"#,
                ))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::new();
            let (html, retries) =
                request_standard_with_retries(&client, Engine::Duckduckgo, "test", || {
                    client.post(server.uri())
                })
                .await
                .unwrap();
            assert_eq!(retries, 0);
            let error = parse_duckduckgo_response(&html).unwrap_err();
            assert!(error.to_string().contains("bot challenge"));
            assert!(error.to_string().contains("--engine bing"));
        }
    }

    #[tokio::test]
    async fn mojeek_http_success_challenge_and_forbidden_remain_distinct() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

        let challenge = include_str!("../tests/fixtures/providers/mojeek-challenge.html");
        for status in [200, 403] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_string(challenge))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::new();
            let response = request_standard_with_retries(&client, Engine::Mojeek, "test", || {
                client.get(server.uri())
            })
            .await;
            let error = if status == 200 {
                let (html, retries) = response.unwrap();
                assert_eq!(retries, 0);
                crate::providers::parse(Engine::Mojeek, &html).unwrap_err()
            } else {
                response.unwrap_err()
            };
            let message = error.to_string();
            assert_eq!(
                provider_error_outcome(&message),
                if status == 200 {
                    "challenge"
                } else {
                    "request_error"
                }
            );
            if status == 403 {
                assert!(message.contains("HTTP 403"));
                assert!(!message.contains("bot challenge"));
            }
        }
    }

    #[test]
    fn parses_redirects_and_canonicalizes() {
        let encoded =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("https://example.com/bing");
        let html = format!(
            r#"<li class="b_algo"><h2><a href="https://www.bing.com/ck/a?u=a1{encoded}">Bing result</a></h2><div class="b_attribution"><cite>example.com/bing</cite></div><div class="b_caption"><p>Bing snippet</p></div></li>"#
        );
        assert_eq!(parse_bing_results(&html)[0].url, "https://example.com/bing");
        assert_eq!(
            canonical_url("HTTPS://Example.COM/path/?utm_source=x&a=1#top"),
            "https://example.com/path?a=1"
        );
    }

    #[test]
    fn parses_yahoo_redirect() {
        let html = r#"<div class="dd algo"><h3><a href="https://r.search.yahoo.com/RU=https%3A%2F%2Fexample.com%2Fyahoo/RK=2/RS=x">Yahoo result</a></h3><div class="compText"><p>Yahoo snippet</p></div></div>"#;
        let result = &parse_yahoo_results(html)[0];
        assert_eq!(result.url, "https://example.com/yahoo");
        assert_eq!(result.display_url, "example.com");
    }

    #[test]
    fn parses_yahoo_mobile_result_without_attribution_in_title() {
        let html = r#"<div class="compTitle p-r"><h3 class="title"><a aria-label="Rust Programming Language" href="https://r.search.yahoo.com/RU=https%3A%2F%2Frust-lang.org%2F/RK=2"><span>Rust Programming Language</span>https://rust-lang.org Rust Programming Language</a></h3></div><div class="compText"><p>Useful Rust result snippet.</p></div>"#;
        let result = &parse_yahoo_results(html)[0];
        assert_eq!(result.title, "Rust Programming Language");
        assert_eq!(result.url, "https://rust-lang.org/");
        assert_eq!(result.snippet, "Useful Rust result snippet.");
    }

    #[test]
    fn merges_provider_buckets_round_robin() {
        let bucket = |engine, label: &str| {
            with_provenance(
                (1..=2)
                    .map(|rank| {
                        SearchResult::parsed(
                            format!("{label}{rank}"),
                            format!("https://example.com/{label}/{rank}"),
                            "example.com".into(),
                            "snippet".into(),
                        )
                    })
                    .collect(),
                engine,
                "query",
            )
        };
        let merged = merge_round_robin(vec![
            bucket(Engine::Duckduckgo, "d"),
            bucket(Engine::Bing, "b"),
            bucket(Engine::Yahoo, "y"),
        ]);
        assert_eq!(
            merged
                .iter()
                .map(|result| result.title.as_str())
                .collect::<Vec<_>>(),
            ["d1", "b1", "y1", "d2", "b2", "y2"]
        );
    }

    #[test]
    fn merging_duplicate_urls_preserves_sources() {
        let first = with_provenance(parse_duckduckgo_results(DDG), Engine::Duckduckgo, "one");
        let second = with_provenance(parse_duckduckgo_results(DDG), Engine::Bing, "two");
        let merged = merge_round_robin(vec![first, second]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sources.len(), 2);
    }

    #[test]
    fn validates_and_deduplicates_dimensions() {
        let options = SearchOptions {
            engines: vec![Engine::Bing, Engine::Bing, Engine::Yahoo],
            ..SearchOptions::default()
        };
        let (queries, engines) = validate_request(
            &[
                "alpha".into(),
                " alpha ".into(),
                String::new(),
                "beta".into(),
            ],
            &options,
        )
        .unwrap();
        assert_eq!(queries, ["alpha", "beta"]);
        assert_eq!(engines, [Engine::Bing, Engine::Yahoo]);
    }

    #[test]
    fn validates_provider_quorum_against_mode_and_engine_count() {
        let fallback = SearchOptions {
            provider_quorum: Some(1),
            ..SearchOptions::default()
        };
        assert!(
            validate_request(&["query".into()], &fallback)
                .unwrap_err()
                .to_string()
                .contains("fanout mode")
        );
        let too_large = SearchOptions {
            engines: vec![Engine::Duckduckgo, Engine::Bing],
            mode: SearchMode::Fanout,
            provider_quorum: Some(3),
            ..SearchOptions::default()
        };
        assert!(
            validate_request(&["query".into()], &too_large)
                .unwrap_err()
                .to_string()
                .contains("between 1 and the 2 selected engines")
        );
    }

    #[tokio::test]
    async fn fanout_quorum_keeps_engine_order_and_cancels_straggler() {
        use std::pin::Pin;

        type Job =
            Pin<Box<dyn Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)> + Send>>;
        let result = |label: &str| {
            SearchResult::parsed(
                label.into(),
                format!("https://example.com/{label}"),
                String::new(),
                "snippet".into(),
            )
        };
        let pending = FuturesUnordered::<Job>::new();
        pending.push(Box::pin(async move { (1, Ok(vec![result("bing")])) }));
        pending.push(Box::pin(async move { (0, Ok(vec![result("duckduckgo")])) }));
        pending.push(Box::pin(async move {
            std::future::pending::<(usize, Result<Vec<SearchResult>, KestrelError>)>().await
        }));

        let (outcomes, cancelled) = collect_fanout(pending, Some(2)).await;
        assert_eq!(cancelled, 1);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].as_ref().unwrap()[0].title, "duckduckgo");
        assert_eq!(outcomes[1].as_ref().unwrap()[0].title, "bing");
    }

    #[test]
    fn outcome_merging_keeps_partial_success_and_rejects_total_failure() {
        let success = with_provenance(parse_duckduckgo_results(DDG), Engine::Duckduckgo, "one");
        let merged = merge_outcomes(
            vec![Ok(success), Err(KestrelError::Search("offline".into()))],
            SearchMode::Fanout,
        )
        .unwrap();
        assert_eq!(merged.len(), 1);
        assert!(
            merge_outcomes(
                vec![Err(KestrelError::Search("offline".into()))],
                SearchMode::Fallback,
            )
            .unwrap_err()
            .to_string()
            .contains("Every search failed")
        );
    }
    #[test]
    fn site_restrictions_respect_host_boundaries_and_query_syntax() {
        let query = "site:Postgresql.org \"EXPLAIN ANALYZE\" BUFFERS";
        assert!(result_allowed(
            query,
            "https://www.postgresql.org/docs/current/sql-explain.html"
        ));
        for url in [
            "https://postgresql.org.evil.test/",
            "https://evilpostgresql.org/",
            "https://evil.test/postgresql.org",
            "javascript:alert(1)",
        ] {
            assert!(!result_allowed(query, url));
        }
        for query in [
            "NOT site:postgresql.org explain",
            "site:postgresql.org OR site:example.org",
            "\"site:postgresql.org\" explain",
            "-site:postgresql.org",
            "site:postgresql.org/docs explain",
        ] {
            assert_eq!(site_domain(query), None);
        }
        assert_ne!(
            canonical_url("https://postgresql.org/docs/16/sql-explain.html"),
            canonical_url("https://postgresql.org/docs/17/sql-explain.html")
        );
    }

    #[tokio::test]
    async fn fallback_continues_after_empty_or_failed_provider() {
        let mut calls = Vec::new();
        let results = collect_fallback(
            "query",
            &[Engine::Bing, Engine::Duckduckgo, Engine::Yahoo],
            |engine| {
                calls.push(engine);
                std::future::ready(match engine {
                    Engine::Bing => Ok(Vec::new()),
                    Engine::Duckduckgo => Err(KestrelError::Search("challenge".into())),
                    _ => Ok(vec![SearchResult::parsed(
                        "useful".into(),
                        "https://example.org".into(),
                        String::new(),
                        String::new(),
                    )]),
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn dropped_provider_preserves_retries_and_censored_backoff() {
        let diagnostics = Arc::new(Mutex::new(vec![ProviderSearchDiagnostic {
            engine: Engine::Bing,
            query: "test".into(),
            elapsed_ms: 0,
            result_count: 0,
            retries: 0,
            success: false,
            outcome: "cancelled_quorum".into(),
            error: None,
            raw_result_count: 0,
            filtered_count: 0,
        }]));
        let recorder = Arc::new(Mutex::new(Recorder::new()));
        let task_recorder = Arc::clone(&recorder);
        let task_diagnostics = Arc::clone(&diagnostics);
        let mut job = Box::pin(async move {
            let _timer = DiagnosticTimer {
                diagnostics: task_diagnostics,
                index: 0,
                started: Instant::now(),
                recorder: Arc::clone(&task_recorder),
                completed: false,
                deadline: false,
                quorum_cancelled: Some(Arc::new(AtomicBool::new(true))),
            };
            PROVIDER_RECORDER
                .scope(task_recorder, async {
                    record_attempt();
                    record_phase(Phase::Backoff);
                    record_attempt();
                    record_phase(Phase::Backoff);
                    std::future::pending::<()>().await;
                })
                .await;
        });
        assert!(futures_util::poll!(&mut job).is_pending());
        drop(job);
        let entries = diagnostics.lock().unwrap();
        assert_eq!(entries[0].retries, 1);
        assert_eq!(entries[0].outcome, "cancelled_quorum");
        let snapshot = recorder.lock().unwrap().finish(false);
        assert_eq!(snapshot.send_attempts, 2);
        assert_eq!(snapshot.cancellation_phase, Some(Phase::Backoff));
    }

    #[tokio::test]
    async fn deadline_includes_provider_queue_and_records_reason() {
        let clients = SearchClients::new(&[Engine::Bing]).unwrap();
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let result = run_one(
            "query",
            Engine::Bing,
            &clients,
            Arc::new(Semaphore::new(0)),
            Arc::clone(&diagnostics),
            "",
            TimeFilter::Any,
            Some(tokio::time::Instant::now() + Duration::from_millis(5)),
            None,
        )
        .await;
        assert!(result.is_err());
        let entries = diagnostics.lock().unwrap();
        assert_eq!(entries[0].outcome, "deadline");
        assert!(!entries[0].success);
    }

    #[test]
    fn additional_html_parsers_reject_shells_and_accept_explicit_empty() {
        for engine in [Engine::Bing, Engine::Yahoo] {
            assert!(parse_provider_response(engine, "<html><nav>Home</nav></html>").is_err());
            assert!(parse_provider_response(engine, "<form id='captcha'></form>").is_err());
        }
        assert!(
            parse_provider_response(Engine::Bing, "<li class='b_no'>No results</li>")
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_provider_response(Engine::Yahoo, "<div class='msgNoResults'>No results</div>")
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn filtering_preserves_original_provider_rank() {
        let mut retained = SearchResult::parsed(
            "Docs".into(),
            "https://postgresql.org/docs/".into(),
            String::new(),
            String::new(),
        );
        retained.engine_rank = Some(4);
        let results = with_provenance(vec![retained], Engine::Bing, "site:postgresql.org explain");
        assert_eq!(results[0].engine_rank, Some(4));
        assert_eq!(results[0].sources[0].rank, 4);
    }
}

#[cfg(test)]
#[path = "search/diagnostic_tests.rs"]
mod diagnostic_tests;
