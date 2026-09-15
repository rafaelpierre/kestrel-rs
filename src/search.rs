//! Public search façade and provider fanout orchestration.

use opentelemetry::trace::FutureExt;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(test)]
use futures_util::StreamExt;
use futures_util::{future::join_all, stream::FuturesUnordered};
use tokio::sync::Semaphore;
#[cfg(any(test, feature = "test-fixtures"))]
use url::Url;

use crate::model::{
    Engine, ProviderSearchDiagnostic, SearchOptions, SearchReport, SearchResult, TimeFilter,
};

use crate::provider_diagnostics::{Phase, Recorder};

mod discovery;
mod failure;
use crate::ProviderOutcome;
pub(crate) use failure::ProviderFailure;
pub(crate) mod parsing;
mod results;
mod streaming;
pub(crate) mod transport;

pub use crate::error::KestrelError;
#[cfg(test)]
use crate::providers::html::{element_text, selector};
#[cfg(any(test, feature = "test-fixtures"))]
use crate::providers::response::extract_dispatched;
use crate::providers::{
    bing::search_bing, duckduckgo::search_duckduckgo, search_additional, yahoo::search_yahoo,
};
#[cfg(test)]
use base64::Engine as _;
pub(crate) use results::canonical_url;
use results::result_key;
use results::{filter_response, merge_outcomes, with_provenance};
#[cfg(test)]
use results::{merge_round_robin, site_domain};
pub use transport::MAX_PROVIDER_RESPONSE_BYTES;
use transport::SEARCH_TIMEOUT;
#[cfg(any(test, feature = "test-fixtures"))]
use transport::{
    request_impersonated_with_retries, request_standard_with_retries, request_yahoo_with_retries,
};

#[cfg(test)]
use crate::providers::bing::{
    bing_impersonated_request, bing_request, parse_bing_document, parse_bing_results,
};
#[cfg(test)]
use crate::providers::duckduckgo::{
    duckduckgo_request, parse_duckduckgo_response, parse_duckduckgo_results,
};
#[cfg(test)]
use crate::providers::response::{
    ParsedResponse, RESPONSE_PARSES, classify_challenge, extract_completed,
    parse_provider_response, process_completed, retain_body,
};
#[cfg(test)]
use crate::providers::yahoo::{parse_yahoo_results, yahoo_request};
#[cfg(test)]
use crate::search::results::{normalize_provider_results, result_allowed};
#[cfg(test)]
use crate::search::transport::{
    ProviderBody, TEST_BACKOFF_ENTERED, TEST_RETRY_DELAY, is_tls_error, read_standard_body,
};
#[cfg(test)]
use scraper::Html;

// Per-query cancellation reason, shared with provider lifecycle guards.
const FANOUT_RUNNING: u8 = 0;
const FANOUT_MIN_RESULTS: u8 = 2;

tokio::task_local! {
    static DISCOVERY_ATTEMPT: usize;
    static ATTEMPT_STATES: Arc<Mutex<HashMap<Engine, ProviderOutcome>>>;
    static RATE_LIMITED: Arc<std::sync::atomic::AtomicBool>;
    static DISCOVERY_SEQUENCE: std::sync::atomic::AtomicU64;
    static PROVIDER_RECORDER: Arc<Mutex<Recorder>>;
    static DIAGNOSTIC_RUN_ID: String;
    static PROVIDER_DIAGNOSTIC: (Arc<Mutex<Vec<ProviderSearchDiagnostic>>>, usize);
}

fn record_headers(status: u16, retry_after: Option<String>) {
    if status == 429 || retry_after.is_some() {
        let _ = RATE_LIMITED.try_with(|state| state.store(true, Ordering::Relaxed));
    }
    observe(|r| r.headers(status, retry_after));
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

#[derive(Clone)]
pub(crate) struct SearchClients {
    pub(crate) standard: crate::http_client::Client,
    pub(crate) bing_transport: crate::BingTransport,
    pub(crate) bing: Option<primp::Client>,
    pub(crate) yahoo: Option<primp::Client>,
    pub(crate) parsers: parsing::ParserPool,
}

impl SearchClients {
    fn validate_engines(&self, engines: &[Engine]) -> Result<(), KestrelError> {
        if engines.contains(&Engine::Bing)
            && self.bing.is_none()
            && matches!(self.bing_transport, crate::BingTransport::Impersonated)
        {
            return Err(KestrelError::InvalidRequest(
                "Bing impersonated transport was not enabled when constructing this client".into(),
            ));
        }
        if engines.contains(&Engine::Yahoo) && self.yahoo.is_none() {
            return Err(KestrelError::InvalidRequest(
                "Yahoo transport was not enabled when constructing this client".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn new(engines: &[Engine]) -> Result<Self, KestrelError> {
        Self::with_transport(engines, &crate::TransportOptions::default())
    }

    pub(crate) fn with_transport(
        engines: &[Engine],
        transport: &crate::TransportOptions,
    ) -> Result<Self, KestrelError> {
        transport.validate()?;
        let profile = crate::http_client::BrowserProfile::random();
        let standard = crate::http_client::Client::new(profile, transport, Some(SEARCH_TIMEOUT))?;
        let bing = (engines.contains(&Engine::Bing)
            && matches!(transport.bing_transport, crate::BingTransport::Impersonated))
        .then(|| impersonated_search_client(crate::http_client::BrowserProfile::bing(), transport));
        let yahoo = engines
            .contains(&Engine::Yahoo)
            .then(|| impersonated_search_client(profile, transport));
        crate::benchmarking::capture_headers("search", &profile.headers());
        Ok(Self {
            standard,
            bing_transport: transport.bing_transport,
            bing: bing.transpose()?,
            yahoo: yahoo.transpose()?,
            parsers: parsing::ParserPool::default(),
        })
    }
}

fn impersonated_search_client(
    profile: crate::http_client::BrowserProfile,
    transport: &crate::TransportOptions,
) -> Result<primp::Client, primp::Error> {
    let mut client = crate::http_client::impersonated_builder(profile, transport)
        .timeout(SEARCH_TIMEOUT)
        .build()?;
    *client.headers_mut() = profile.headers();
    Ok(client)
}

fn validate_query(query: &str) -> Result<(), KestrelError> {
    if query.trim().is_empty() {
        return Err(KestrelError::InvalidRequest(
            "At least one non-empty query is required".into(),
        ));
    }
    Ok(())
}

/// Search one provider asynchronously.
pub async fn search(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
) -> Result<Vec<SearchResult>, KestrelError> {
    validate_query(query)?;
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
    crate::telemetry::scope_result("kestrel.search", async {
        validate_query(query)?;
        clients.validate_engines(&[engine])?;
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
    })
    .await
}

pub(crate) struct ProviderResponse {
    pub(crate) results: Vec<SearchResult>,
    pub(crate) retries: usize,
    pub(crate) raw_result_count: usize,
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
    crate::telemetry::scope_result("kestrel.search", async {
        let (queries, engines) = validate_request(queries, options)?;
        let clients = SearchClients::new(&engines)?;
        search_many_with_clients_detailed(queries, engines, options, &clients, None).await
    })
    .await
}

pub(crate) async fn search_many_reusing_clients(
    queries: &[String],
    options: &SearchOptions,
    clients: &SearchClients,
    recovery: Option<&crate::SearchRecovery>,
) -> Result<Vec<SearchResult>, KestrelError> {
    Ok(
        search_many_reusing_clients_detailed(queries, options, clients, recovery)
            .await?
            .results,
    )
}

pub(crate) async fn search_many_reusing_clients_detailed(
    queries: &[String],
    options: &SearchOptions,
    clients: &SearchClients,
    recovery: Option<&crate::SearchRecovery>,
) -> Result<SearchReport, KestrelError> {
    crate::telemetry::scope_result("kestrel.search", async {
        let (queries, engines) = validate_request(queries, options)?;
        clients.validate_engines(&engines)?;
        search_many_with_clients_detailed(queries, engines, options, clients, recovery).await
    })
    .await
}

async fn search_many_with_clients_detailed(
    queries: Vec<String>,
    engines: Vec<Engine>,
    options: &SearchOptions,
    clients: &SearchClients,
    recovery: Option<&crate::SearchRecovery>,
) -> Result<SearchReport, KestrelError> {
    DIAGNOSTIC_RUN_ID
        .scope(
            uuid::Uuid::new_v4().to_string(),
            search_many_with_clients_in_run(queries, engines, options, clients, recovery),
        )
        .await
}

async fn search_many_with_clients_in_run(
    queries: Vec<String>,
    engines: Vec<Engine>,
    options: &SearchOptions,
    clients: &SearchClients,
    recovery: Option<&crate::SearchRecovery>,
) -> Result<SearchReport, KestrelError> {
    crate::telemetry::payload("search.input", &queries);
    crate::telemetry::attribute("kestrel.query_syntax", "passthrough");
    crate::telemetry::attribute("kestrel.time_filter", format!("{:?}", options.time_filter));
    crate::telemetry::payload("search.region", &options.region);
    if let Some(budget) = options.search_budget {
        crate::telemetry::attribute("kestrel.search_budget_seconds", budget.as_secs_f64());
    }
    crate::telemetry::attribute("kestrel.search_concurrency", options.max_concurrency as i64);
    crate::telemetry::attribute(
        "kestrel.min_results",
        options.min_results.unwrap_or(5) as i64,
    );
    let semaphore = Arc::new(Semaphore::new(options.max_concurrency));
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let policy = discovery::Policy::new(options.search_budget);
    let overall_deadline = policy
        .overall
        .map(|budget| crate::numeric::deadline("discovery allowance", budget))
        .transpose()?;
    let (progress, writer) = crate::recovery::writer(recovery, overall_deadline);
    let jobs = queries
        .iter()
        .enumerate()
        .map(|(index, query)| {
            DISCOVERY_SEQUENCE.scope(
                std::sync::atomic::AtomicU64::new(0),
                discovery::run(
                    query,
                    index,
                    &engines,
                    options,
                    clients,
                    Arc::clone(&semaphore),
                    Arc::clone(&diagnostics),
                    recovery,
                    progress.clone(),
                    policy,
                    overall_deadline,
                ),
            )
        })
        .collect::<Vec<_>>();
    let collect = async {
        let outcomes = join_all(jobs).await;
        drop(progress);
        outcomes
    };
    let (query_outcomes, ()) = tokio::join!(collect, writer);
    let cancelled = query_outcomes.iter().map(|(_, count)| count).sum();
    let outcomes = query_outcomes
        .into_iter()
        .flat_map(|(outcomes, _)| outcomes)
        .collect();
    let results = crate::telemetry::scope_sync("kestrel.merge", || merge_outcomes(outcomes))
        .map_err(|_| discovery::failure(&diagnostics.lock().expect("diagnostic lock")))?;
    crate::telemetry::results("search.output", &results);
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
    min_results: usize,
    deadline: Option<tokio::time::Instant>,
    progress: Option<crate::recovery::ProgressQueue>,
) -> impl Future<Output = (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)> + 'a {
    let engines = engines.to_vec();
    async move {
        let mut query_span = crate::telemetry::Span::new("kestrel.query");
        let query_context = query_span.context();
        let _query_context = query_context.clone().attach();
        crate::telemetry::payload("query.input", &query);
        drop(_query_context);
        let keys: Vec<_> = engines
            .iter()
            .map(|engine| crate::recovery::UnitKey::new(query, *engine, region, time_filter))
            .collect();
        let mut recovered = std::collections::BTreeMap::new();
        if let Some(queue) = &progress {
            // Relative windows change continuously; until adapters expose a stable anchor,
            // these units conservatively miss on every invocation.
            if time_filter == TimeFilter::Any {
                for (index, key) in keys.iter().enumerate() {
                    let end = deadline
                        .or_else(|| Some(tokio::time::Instant::now() + Duration::from_millis(250)));
                    match crate::numeric::before_deadline(end, queue.store.restore(key)).await {
                        Ok(Ok(snapshot)) => {
                            recovered.insert(index, snapshot);
                        }
                        Ok(Err(reason)) => {
                            eprintln!("[kestrel] Recovery miss: {reason}; requesting provider.")
                        }
                        Err(()) => eprintln!(
                            "[kestrel] Recovery read deadline; requesting only within remaining budget."
                        ),
                    }
                }
            } else {
                eprintln!(
                    "[kestrel] Recovery miss: moving recency window has no stable coverage anchor."
                );
            }
            let unique = recovered
                .values()
                .flat_map(|s| s.records.iter().map(result_key))
                .collect::<HashSet<_>>()
                .len();
            let complete = recovered
                .values()
                .filter(|s| s.state == crate::recovery::State::Complete)
                .count();
            eprintln!(
                "[kestrel] Recovery loaded {} unit(s): {complete} complete, {} incomplete, {unique} unique records.",
                recovered.len(),
                recovered.len() - complete
            );
        }
        let target_reached = {
            recovered
                .values()
                .flat_map(|s| s.records.iter().map(result_key))
                .collect::<HashSet<_>>()
                .len()
                >= min_results
        };
        let pending = FuturesUnordered::new();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let fanout_cancelled = Arc::new(AtomicU8::new(FANOUT_RUNNING));
        for (index, engine) in engines.iter().copied().enumerate() {
            if progress.as_ref().is_some_and(|p| p.store.is_cancelled())
                || target_reached
                || recovered
                    .get(&index)
                    .is_some_and(|s| s.state == crate::recovery::State::Complete)
            {
                eprintln!(
                    "[kestrel] Recovery skipped {engine} request: compatible completion or current minimum satisfied."
                );
                continue;
            }
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
                Some(Arc::clone(&fanout_cancelled)),
            );
            let publisher = streaming::Publisher {
                sender: sender.clone(),
                index,
                engine,
                query: query.to_owned(),
            };
            pending.push(async move {
                let outcome = streaming::PUBLISHER.scope(publisher, job).await;
                (index, outcome)
            });
        }
        drop(sender);
        let progress = progress.map(|queue| (queue, keys));
        async move {
            let output = streaming::collect_replaying(
                pending,
                min_results,
                Some(fanout_cancelled),
                Some(receiver),
                progress,
                deadline,
                recovered,
            )
            .await;
            query_span.finish();
            output
        }
        .with_context(query_context)
        .await
    }
}

#[cfg(test)]
async fn collect_fanout<F>(
    pending: FuturesUnordered<F>,
    min_results: usize,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    collect_fanout_signalled(pending, min_results, None).await
}

#[cfg(test)]
async fn collect_fanout_signalled<F>(
    pending: FuturesUnordered<F>,
    min_results: usize,
    fanout_cancelled: Option<Arc<AtomicU8>>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize)
where
    F: Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>,
{
    streaming::collect(pending, min_results, fanout_cancelled, None).await
}

/// Trim query edges, drop empty queries and retain the first occurrence of each
/// remaining query. Search uses these exact strings for provider provenance;
/// callers composing search with metadata selection should use the same list.
pub fn normalize_queries(queries: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    queries
        .iter()
        .map(|query| query.trim())
        .filter(|query| !query.is_empty() && seen.insert(*query))
        .map(str::to_owned)
        .collect()
}

fn validate_request(
    queries: &[String],
    options: &SearchOptions,
) -> Result<(Vec<String>, Vec<Engine>), KestrelError> {
    let clean_queries = normalize_queries(queries);
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
    crate::numeric::concurrency("max_concurrency", options.max_concurrency)?;
    if let Some(budget) = options.search_budget {
        crate::numeric::duration("search budget", budget)?;
    }
    if options.min_results == Some(0) {
        return Err(KestrelError::InvalidRequest(
            "min_results must be at least 1".into(),
        ));
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
    fanout_cancelled: Option<Arc<AtomicU8>>,
) -> impl Future<Output = Result<Vec<SearchResult>, KestrelError>> + 'a {
    let job = run_one_job(
        query,
        engine,
        semaphore,
        diagnostics,
        deadline,
        fanout_cancelled,
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
    fanout_cancelled: Option<Arc<AtomicU8>>,
    provider: impl Future<Output = Result<ProviderResponse, ProviderFailure>> + 'a,
) -> impl Future<Output = Result<Vec<SearchResult>, KestrelError>> + 'a {
    let started = Instant::now();
    let index = {
        let mut entries = diagnostics.lock().expect("diagnostic lock");
        let index = entries.len();
        entries.push(ProviderSearchDiagnostic {
            discovery_attempt: DISCOVERY_ATTEMPT.try_with(|a| *a).unwrap_or(1),
            engine,
            query: query.to_owned(),
            elapsed_ms: 0,
            result_count: 0,
            retries: 0,
            success: false,
            outcome: ProviderOutcome::CancelledCaller.as_str().into(),
            error: None,
            raw_result_count: 0,
            filtered_count: 0,
        });
        index
    };
    // Also records elapsed time when the result minimum drops this future mid-request.
    let run_id = DIAGNOSTIC_RUN_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let telemetry = crate::telemetry::Span::new("kestrel.provider");
    telemetry.attribute("kestrel.provider", engine.to_string());
    let context = telemetry.context();
    let recorder = {
        let _context = context.clone().attach();
        crate::telemetry::payload("input", &query);
        Arc::new(Mutex::new(Recorder::with_run(run_id)))
    };
    let rate_limited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timer = DiagnosticTimer {
        engine,
        state: ProviderOutcome::CancelledCaller,
        states: ATTEMPT_STATES.try_with(Arc::clone).ok(),
        diagnostics: Arc::clone(&diagnostics),
        index,
        started,
        recorder: Arc::clone(&recorder),
        completed: false,
        deadline: false,
        fanout_cancelled,
        telemetry,
    };
    async move {
        let mut timer = timer;
        recorder
            .lock()
            .expect("recorder lock")
            .transition(Phase::Queue);
        let job = RATE_LIMITED.scope(
            Arc::clone(&rate_limited),
            PROVIDER_RECORDER.scope(Arc::clone(&recorder), async {
                let _permit = semaphore.acquire().await.expect("semaphore remains open");
                record_phase(Phase::Processing);
                PROVIDER_DIAGNOSTIC
                    .scope((Arc::clone(&diagnostics), index), provider)
                    .await
            }),
        );
        let outcome = match deadline {
            Some(deadline) if deadline <= tokio::time::Instant::now() => {
                timer.deadline = true;
                Err(KestrelError::SearchDeadline.into())
            }
            Some(deadline) => tokio::time::timeout_at(deadline, job)
                .await
                .unwrap_or_else(|_| {
                    timer.deadline = true;
                    Err(KestrelError::SearchDeadline.into())
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
                    timer.state = if response.results.is_empty() {
                        if response.raw_result_count > response.results.len() {
                            ProviderOutcome::FilteredEmpty
                        } else {
                            ProviderOutcome::Empty
                        }
                    } else {
                        ProviderOutcome::Results
                    };
                }
                Err(error) => {
                    let message = error.to_string();
                    timer.state = if error.kind == ProviderOutcome::Deadline
                        && rate_limited.load(Ordering::Relaxed)
                    {
                        ProviderOutcome::RateLimitedDeadline
                    } else {
                        error.kind
                    };
                    entry.error = Some(message);
                }
            }
            entry.outcome = timer.state.as_str().into();
        }
        if let Ok(response) = &outcome {
            crate::telemetry::results("provider.results", &response.results);
        }
        outcome
            .map(|response| response.results)
            .map_err(ProviderFailure::into_public)
    }
    .with_context(context)
}

struct DiagnosticTimer {
    engine: Engine,
    state: ProviderOutcome,
    states: Option<Arc<Mutex<HashMap<Engine, ProviderOutcome>>>>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    index: usize,
    started: Instant,
    recorder: Arc<Mutex<Recorder>>,
    completed: bool,
    deadline: bool,
    fanout_cancelled: Option<Arc<AtomicU8>>,
    telemetry: crate::telemetry::Span,
}
impl Drop for DiagnosticTimer {
    fn drop(&mut self) {
        let cancelled = !self.completed || self.deadline;
        let diagnostic = self.diagnostics.lock().ok().map(|mut entries| {
            let entry = &mut entries[self.index];
            entry.elapsed_ms = elapsed_millis(self.started);
            if !self.completed {
                self.state = match self
                    .fanout_cancelled
                    .as_ref()
                    .map_or(FANOUT_RUNNING, |s| s.load(Ordering::Relaxed))
                {
                    FANOUT_MIN_RESULTS => ProviderOutcome::CancelledMinResults,
                    _ => ProviderOutcome::CancelledCaller,
                };
                entry.outcome = self.state.as_str().into();
            }
            if let Ok(recorder) = self.recorder.lock() {
                entry.retries = recorder.retries();
            }
            entry.clone()
        });
        if let Some(states) = &self.states {
            states
                .lock()
                .expect("attempt state lock")
                .insert(self.engine, self.state);
        }
        if let Some(diagnostic) = diagnostic {
            let _context = self.telemetry.context().attach();
            self.telemetry
                .attribute("kestrel.outcome", diagnostic.outcome.clone());
            self.telemetry.attribute("kestrel.cancelled", cancelled);
            if !diagnostic.success && !cancelled {
                crate::telemetry::error("provider_failed");
            }
            self.telemetry.finish();
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

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

async fn run_provider(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
    clients: &SearchClients,
) -> Result<ProviderResponse, ProviderFailure> {
    parsing::POOL
        .scope(
            clients.parsers.clone(),
            run_provider_inner(query, engine, region, time_filter, clients),
        )
        .await
}

async fn run_provider_inner(
    query: &str,
    engine: Engine,
    region: &str,
    time_filter: TimeFilter,
    clients: &SearchClients,
) -> Result<ProviderResponse, ProviderFailure> {
    #[cfg(test)]
    if let Ok(fixture) = discovery::tests::FIXTURE.try_with(Clone::clone) {
        return fixture.respond(query, engine).await;
    }
    #[cfg(any(test, feature = "test-fixtures"))]
    if let Ok(endpoint) = std::env::var("KESTREL_TEST_PROVIDER_ENDPOINT") {
        let mut endpoint =
            Url::parse(&endpoint).map_err(|e| KestrelError::InvalidRequest(e.to_string()))?;
        if endpoint.scheme() != "http"
            || !matches!(endpoint.host_str(), Some("127.0.0.1" | "[::1]"))
        {
            return Err(KestrelError::InvalidRequest(
                "test fixture endpoint must be loopback HTTP".into(),
            )
            .into());
        }
        endpoint.set_path(&format!("/{engine}"));
        let (results, retries) = if engine == Engine::Yahoo {
            request_yahoo_with_retries(query, extract_dispatched, || {
                clients
                    .yahoo
                    .as_ref()
                    .expect("Yahoo client requested")
                    .get(endpoint.as_str())
                    .query(&[("q", query)])
            })
            .await?
        } else if engine == Engine::Bing && clients.bing.is_some() {
            request_impersonated_with_retries(engine, query, extract_dispatched, || {
                clients
                    .bing
                    .as_ref()
                    .expect("Bing impersonated client requested")
                    .get(endpoint.as_str())
                    .query(&[("q", query)])
            })
            .await?
        } else {
            request_standard_with_retries(
                &clients.standard,
                engine,
                query,
                extract_dispatched,
                || {
                    clients
                        .standard
                        .get(endpoint.as_str())
                        .query(&[("q", query)])
                },
            )
            .await?
        };
        let mut response = ProviderResponse {
            results: results?,
            retries,
            raw_result_count: 0,
        };
        filter_response(query, &mut response);
        return Ok(response);
    }
    let mut result = match engine {
        Engine::Duckduckgo => {
            search_duckduckgo(query, region, time_filter, &clients.standard).await
        }
        Engine::Bing => {
            search_bing(
                query,
                region,
                time_filter,
                &clients.standard,
                clients.bing.as_ref(),
            )
            .await
        }
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
        filter_response(query, response);
        #[cfg(test)]
        streaming::probe::results(engine, &response.results);
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

#[cfg(test)]
mod parse_once_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const DDG: &str = r#"
    <div class="result results_links results_links_deep web-result">
      <h2 class="result__title"><a class="result__a" href="https://example.com">Example</a></h2>
      <a class="result__url">example.com</a><a class="result__snippet">A useful result</a>
    </div><div class="result results_links results_links_deep web-result"><h2></h2></div>"#;

    fn assert_oversized(error: impl Into<ProviderFailure>, engine: Engine, expected_status: u16) {
        let error = error.into().into_public();
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
    fn numeric_search_concurrency_accepts_semaphore_boundary() {
        let _telemetry = crate::telemetry::test_export_guard();
        let options = SearchOptions {
            max_concurrency: Semaphore::MAX_PERMITS,
            search_budget: Some(Duration::from_nanos(1)),
            ..Default::default()
        };
        assert!(validate_request(&["test".into()], &options).is_ok());
    }

    #[test]
    fn provider_buffer_checks_capacity_before_appending() {
        let _telemetry = crate::telemetry::test_export_guard();
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
    ) -> Result<(String, usize), ProviderFailure> {
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
                    request_yahoo_with_retries("test", retain_body, || client.get(&url)).await
                } else {
                    let client: crate::http_client::Client = reqwest::Client::builder()
                        .no_proxy()
                        .build()
                        .unwrap()
                        .into();
                    request_standard_with_retries(
                        &client,
                        Engine::Bing,
                        "test",
                        retain_body,
                        || client.get(&url),
                    )
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
        let _telemetry = crate::telemetry::test_export_guard();
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
        let _telemetry = crate::telemetry::test_export_guard();
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
    async fn server_retry_guidance_cannot_be_bypassed_at_deadline() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        for yahoo in [false, true] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(429)
                        .insert_header("retry-after", "1")
                        .set_body_string("rate limited"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let engine = if yahoo { Engine::Yahoo } else { Engine::Bing };
            let clients = SearchClients::new(&[engine]).unwrap();
            let diagnostics = Arc::new(Mutex::new(Vec::new()));
            let request = async {
                if yahoo {
                    request_yahoo_with_retries("q", retain_body, || {
                        clients.yahoo.as_ref().unwrap().get(server.uri())
                    })
                    .await?;
                } else {
                    request_standard_with_retries(
                        &clients.standard,
                        engine,
                        "q",
                        retain_body,
                        || clients.standard.get(server.uri()),
                    )
                    .await?;
                }
                Ok(ProviderResponse {
                    results: Vec::new(),
                    raw_result_count: 0,
                    retries: 0,
                })
            };
            let result = run_one_job(
                "q",
                engine,
                Arc::new(Semaphore::new(1)),
                diagnostics.clone(),
                Some(tokio::time::Instant::now() + Duration::from_millis(100)),
                None,
                request,
            )
            .await;
            assert!(matches!(result, Err(KestrelError::SearchDeadline)));
            assert_eq!(
                diagnostics.lock().unwrap()[0].outcome,
                "rate_limited_deadline"
            );
            server.verify().await;
        }
    }

    #[tokio::test]
    async fn provider_http_errors_keep_retry_policy_under_limit() {
        let _telemetry = crate::telemetry::test_export_guard();
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
                    request_yahoo_with_retries("test", retain_body, || client.get(server.uri()))
                        .await
                } else {
                    let client: crate::http_client::Client = reqwest::Client::builder()
                        .no_proxy()
                        .build()
                        .unwrap()
                        .into();
                    request_standard_with_retries(
                        &client,
                        Engine::Bing,
                        "test",
                        retain_body,
                        || client.get(server.uri()),
                    )
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
        let _telemetry = crate::telemetry::test_export_guard();
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
                            request_yahoo_with_retries("test", retain_body, || {
                                client.get(&url).timeout(Duration::from_millis(300))
                            })
                            .await
                        } else {
                            let client: crate::http_client::Client = reqwest::Client::builder()
                                .no_proxy()
                                .build()
                                .unwrap()
                                .into();
                            request_standard_with_retries(
                                &client,
                                Engine::Bing,
                                "test",
                                retain_body,
                                || client.get(&url).timeout(Duration::from_millis(300)),
                            )
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
                            result.unwrap_err().into_public(),
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

    #[test]
    fn oversized_provider_preserves_partial_success() {
        let _telemetry = crate::telemetry::test_export_guard();
        let oversized = || KestrelError::ProviderResponseTooLarge {
            engine: Engine::Bing,
            limit_bytes: MAX_PROVIDER_RESPONSE_BYTES,
            status: 200,
        };
        let results = parse_duckduckgo_response(DDG).unwrap();
        let merged = merge_outcomes(vec![Err(oversized()), Ok(results)]).unwrap();
        assert_eq!(merged.len(), 1);
        let error = merge_outcomes(vec![Err(oversized())])
            .unwrap_err()
            .to_string();
        assert!(error.contains("bing response exceeds 4194304 decoded bytes (HTTP 200)"));
    }

    #[test]
    fn parses_duckduckgo() {
        let _telemetry = crate::telemetry::test_export_guard();
        let results = parse_duckduckgo_response(DDG).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example");
        assert_eq!(results[0].snippet, "A useful result");
    }

    #[test]
    fn duckduckgo_distinguishes_empty_results_from_unrecognized_pages() {
        let _telemetry = crate::telemetry::test_export_guard();
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
        let _telemetry = crate::telemetry::test_export_guard();
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
            let client: crate::http_client::Client = reqwest::Client::new().into();
            let (html, retries) = request_standard_with_retries(
                &client,
                Engine::Duckduckgo,
                "test",
                retain_body,
                || client.post(server.uri()),
            )
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
        let _telemetry = crate::telemetry::test_export_guard();
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

        let challenge = include_str!("../tests/fixtures/providers/mojeek-challenge.html");
        for status in [200, 403] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_string(challenge))
                .expect(1)
                .mount(&server)
                .await;
            let client: crate::http_client::Client = reqwest::Client::new().into();
            let response =
                request_standard_with_retries(&client, Engine::Mojeek, "test", retain_body, || {
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
            assert_eq!(error.kind, ProviderOutcome::Challenge);
            if status == 403 {
                assert!(message.contains("HTTP 403"));
                assert!(message.contains("bot challenge"));
            }
        }
    }

    const BING_UNRELATED: &str = include_str!("../tests/fixtures/providers/bing-unrelated.html");

    #[tokio::test]
    async fn single_provider_apis_reject_blank_queries() {
        let _telemetry = crate::telemetry::test_export_guard();
        let client = crate::KestrelClient::new().unwrap();
        for query in ["", " ", "\t\n", "\u{2003}"] {
            assert!(matches!(
                search(query, Engine::Bing, "", TimeFilter::Any).await,
                Err(KestrelError::InvalidRequest(_))
            ));
            assert!(matches!(
                client
                    .search(query, Engine::Bing, "", TimeFilter::Any)
                    .await,
                Err(KestrelError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn blocking_search_rejects_blank_queries() {
        let _telemetry = crate::telemetry::test_export_guard();
        for query in ["", " ", "\t\n", "\u{2003}"] {
            assert!(matches!(
                search_blocking(query, Engine::Bing, "", TimeFilter::Any),
                Err(KestrelError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn provider_requests_preserve_query_text() {
        let _telemetry = crate::telemetry::test_export_guard();
        let standard: crate::http_client::Client = reqwest::Client::new().into();
        let yahoo = primp::Client::builder().build().unwrap();
        for query in [
            r#""machine learning""#,
            "machine AND learning",
            "machine learning",
            "(Rust OR Python) NOT game",
            "site:example.com C++ café 日本語 & x=1+2%",
            r#""say \"hello\"""#,
        ] {
            let ddg = duckduckgo_request(&standard, query, "uk-en", TimeFilter::W)
                .build()
                .unwrap();
            let pairs: HashMap<_, _> =
                url::form_urlencoded::parse(ddg.body().unwrap().as_bytes().unwrap())
                    .into_owned()
                    .collect();
            assert_eq!(pairs["q"], query);
            assert_eq!(pairs["kl"], "uk-en");
            assert_eq!(pairs["df"], "w");
            let request = yahoo_request(&yahoo, query, "uk-en", TimeFilter::W)
                .build()
                .unwrap();
            let pairs: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
            assert_eq!(pairs["p"], query);
            assert_eq!(pairs["vl"], "uk-en");
            assert_eq!(pairs["btf"], "w");
        }
    }

    #[test]
    fn default_query_mode_retains_incomplete_metadata_for_all_providers() {
        let _telemetry = crate::telemetry::test_export_guard();
        for query in [
            "machine learning",
            "\"machine learning\"",
            "filetype:pdf a OR b",
            "learning AND",
        ] {
            for engine in [
                Engine::Duckduckgo,
                Engine::Bing,
                Engine::Yahoo,
                Engine::Dogpile,
                Engine::Ecosia,
                Engine::Swisscows,
                Engine::Yep,
                Engine::Qwant,
                Engine::Mojeek,
            ] {
                let mut response = ProviderResponse {
                    results: vec![SearchResult::parsed(
                        "An introduction".into(),
                        "https://example.com/article".into(),
                        String::new(),
                        String::new(),
                    )],
                    retries: 0,
                    raw_result_count: 0,
                };
                filter_response(query, &mut response);
                assert_eq!(response.results.len(), 1, "{engine}: {query}");
            }
        }
    }

    #[test]
    fn bing_request_preserves_complete_query_and_region() {
        let _telemetry = crate::telemetry::test_export_guard();
        let client: crate::http_client::Client = reqwest::Client::new().into();
        let impersonated = primp::Client::builder().no_proxy().build().unwrap();
        for query in [
            "python uv guide",
            "literal %20 + repeated  spaces",
            "\"machine learning\"",
            "machine AND learning",
            "machine learning",
            "why does the moon cause ocean tides",
            "Rust E0382 use of moved value fix borrowing clone",
            "tokio watch Receiver borrow_and_update changed deadlock Ref across await",
            "\"use of moved value\" borrowing",
            "site:docs.rs/tokio \"borrow_and_update\"",
            "C++ & Rust café 日本語 #ownership? x=1+2%",
        ] {
            for (region, country) in [("", None), ("gb-en", Some("gb")), ("us", Some("us"))] {
                let request = bing_request(&client, query, region).build().unwrap();
                let browser_request = bing_impersonated_request(&impersonated, query, region)
                    .build()
                    .unwrap();
                assert_eq!(request.method(), reqwest::Method::GET);
                assert_eq!(browser_request.url(), request.url());
                assert_eq!(request.url().scheme(), "https");
                assert_eq!(request.url().host_str(), Some("www.bing.com"));
                assert_eq!(request.url().path(), "/search");
                assert_eq!(request.url().fragment(), None);
                let encoded = request.url().query().unwrap();
                assert!(!encoded.contains('+'), "{query}: {encoded}");
                assert!(encoded.contains("%20"), "{query}: {encoded}");
                if query == "python uv guide" && region.is_empty() {
                    assert_eq!(encoded, "q=python%20uv%20guide");
                }
                if query.contains('+') {
                    assert!(encoded.contains("%2B"), "{query}: {encoded}");
                }
                let pairs: Vec<_> = request.url().query_pairs().into_owned().collect();
                let mut expected = vec![("q".to_owned(), query.to_owned())];
                if let Some(country) = country {
                    expected.push(("cc".to_owned(), country.to_owned()));
                }
                assert_eq!(pairs, expected);
            }
        }
    }

    #[test]
    fn bing_query_echo_does_not_replace_response_entries() {
        let _telemetry = crate::telemetry::test_export_guard();
        let results = parse_provider_response(Engine::Bing, BING_UNRELATED).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Why — dictionary definition");
        assert_eq!(results[0].url, "https://example.com/dictionary/why");
        assert_eq!(results[0].snippet, "The meaning of why: for what reason.");
        assert_eq!(results[1].title, "Why: music video");
        assert_eq!(results[1].url, "https://example.org/music/why");
        assert_eq!(results[1].snippet, "Watch the official music video.");
    }

    // Native passthrough still accepts provider results without lexical checks.
    #[tokio::test]
    async fn native_bing_unrelated_results_satisfy_explicit_minimum() {
        let _telemetry = crate::telemetry::test_export_guard();
        type Outcome = (usize, Result<Vec<SearchResult>, KestrelError>);
        let pending: FuturesUnordered<futures_util::future::BoxFuture<'static, Outcome>> =
            FuturesUnordered::new();
        pending.push(Box::pin(async {
            let mut response = ProviderResponse {
                results: parse_provider_response(Engine::Bing, BING_UNRELATED).unwrap(),
                retries: 0,
                raw_result_count: 0,
            };
            let query = "why does the moon cause ocean tides";
            filter_response(query, &mut response);
            (0, Ok(response.results))
        }));
        pending.push(Box::pin(std::future::pending()));
        let (completed, cancelled) = collect_fanout(pending, 1).await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].as_ref().unwrap().len(), 2);
        assert_eq!(cancelled, 1);
    }

    #[test]
    fn parses_redirects_and_canonicalizes() {
        let _telemetry = crate::telemetry::test_export_guard();
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
        let _telemetry = crate::telemetry::test_export_guard();
        let html = r#"<div class="dd algo"><h3><a href="https://r.search.yahoo.com/RU=https%3A%2F%2Fexample.com%2Fyahoo/RK=2/RS=x">Yahoo result</a></h3><div class="compText"><p>Yahoo snippet</p></div></div>"#;
        let result = &parse_yahoo_results(html)[0];
        assert_eq!(result.url, "https://example.com/yahoo");
        assert_eq!(result.display_url, "example.com");
    }

    #[test]
    fn parses_yahoo_mobile_result_without_attribution_in_title() {
        let _telemetry = crate::telemetry::test_export_guard();
        let html = r#"<div class="compTitle p-r"><h3 class="title"><a aria-label="Rust Programming Language" href="https://r.search.yahoo.com/RU=https%3A%2F%2Frust-lang.org%2F/RK=2"><span>Rust Programming Language</span>https://rust-lang.org Rust Programming Language</a></h3></div><div class="compText"><p>Useful Rust result snippet.</p></div>"#;
        let result = &parse_yahoo_results(html)[0];
        assert_eq!(result.title, "Rust Programming Language");
        assert_eq!(result.url, "https://rust-lang.org/");
        assert_eq!(result.snippet, "Useful Rust result snippet.");
    }

    #[test]
    fn merges_provider_buckets_round_robin() {
        let _telemetry = crate::telemetry::test_export_guard();
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
        let _telemetry = crate::telemetry::test_export_guard();
        let first = with_provenance(parse_duckduckgo_results(DDG), Engine::Duckduckgo, "one");
        let second = with_provenance(parse_duckduckgo_results(DDG), Engine::Bing, "two");
        let merged = merge_round_robin(vec![first, second]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sources.len(), 2);
    }

    #[test]
    fn validates_and_deduplicates_dimensions() {
        let _telemetry = crate::telemetry::test_export_guard();
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
    fn provider_quorum_is_ignored_by_result_count_fanout() {
        let _telemetry = crate::telemetry::test_export_guard();
        for quorum in [0, 3] {
            let options = SearchOptions {
                provider_quorum: Some(quorum),
                ..SearchOptions::default()
            };
            assert!(validate_request(&["query".into()], &options).is_ok());
        }
    }

    #[test]
    fn legacy_quorum_diagnostics_still_deserialize() {
        let legacy = serde_json::json!({
            "engine": "bing", "query": "q", "elapsed_ms": 1,
            "result_count": 0, "retries": 0, "success": false,
            "outcome": "cancelled_quorum", "error": null,
            "raw_result_count": 0, "filtered_count": 0
        });
        let diagnostic: ProviderSearchDiagnostic = serde_json::from_value(legacy).unwrap();
        assert_eq!(diagnostic.outcome, "cancelled_quorum");
        assert_eq!(
            serde_json::to_value(diagnostic).unwrap()["outcome"],
            "cancelled_quorum"
        );
    }

    #[tokio::test]
    async fn fanout_minimum_keeps_engine_order_and_cancels_straggler() {
        let _telemetry = crate::telemetry::test_export_guard();
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

        let (outcomes, cancelled) = collect_fanout(pending, 2).await;
        assert_eq!(cancelled, 1);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].as_ref().unwrap()[0].title, "duckduckgo");
        assert_eq!(outcomes[1].as_ref().unwrap()[0].title, "bing");
    }

    #[test]
    fn outcome_merging_keeps_partial_success_and_rejects_total_failure() {
        let _telemetry = crate::telemetry::test_export_guard();
        let success = with_provenance(parse_duckduckgo_results(DDG), Engine::Duckduckgo, "one");
        let merged = merge_outcomes(vec![
            Ok(success),
            Err(KestrelError::Search("offline".into())),
        ])
        .unwrap();
        assert_eq!(merged.len(), 1);
        assert!(
            merge_outcomes(vec![Err(KestrelError::Search("offline".into()))])
                .unwrap_err()
                .to_string()
                .contains("Every search failed")
        );
    }
    #[test]
    fn site_restrictions_respect_host_boundaries_and_query_syntax() {
        let _telemetry = crate::telemetry::test_export_guard();
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
    async fn fanout_minimum_ignores_empty_and_failed_providers() {
        let _telemetry = crate::telemetry::test_export_guard();
        let pending = FuturesUnordered::new();
        for (index, outcome) in [
            Ok(Vec::new()),
            Err(KestrelError::Search("challenge".into())),
            Ok(parse_duckduckgo_response(DDG).unwrap()),
        ]
        .into_iter()
        .enumerate()
        {
            pending.push(std::future::ready((index, outcome)));
        }
        let (outcomes, cancelled) = collect_fanout(pending, 1).await;
        assert_eq!(outcomes.len(), 3);
        assert_eq!(cancelled, 0);
        assert_eq!(merge_outcomes(outcomes).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fanout_polls_all_providers_and_supports_single_provider() {
        let _telemetry = crate::telemetry::test_export_guard();
        for count in [1, 3] {
            let barrier = Arc::new(tokio::sync::Barrier::new(count));
            let pending = FuturesUnordered::new();
            for index in 0..count {
                let barrier = Arc::clone(&barrier);
                pending.push(async move {
                    barrier.wait().await;
                    (
                        index,
                        Ok(vec![SearchResult::parsed(
                            format!("provider {index}"),
                            format!("https://example.org/{index}"),
                            String::new(),
                            String::new(),
                        )]),
                    )
                });
            }
            let (outcomes, cancelled) =
                tokio::time::timeout(Duration::from_secs(1), collect_fanout(pending, 5))
                    .await
                    .expect("all providers must be polled concurrently");
            assert_eq!(cancelled, 0);
            let results = merge_outcomes(outcomes).unwrap();
            assert_eq!(results.len(), count);
            for (index, result) in results.iter().enumerate() {
                assert_eq!(result.title, format!("provider {index}"));
            }
        }
    }

    #[tokio::test]
    async fn dropped_provider_preserves_retries_and_censored_backoff() {
        let _telemetry = crate::telemetry::test_export_guard();
        let diagnostics = Arc::new(Mutex::new(vec![ProviderSearchDiagnostic {
            discovery_attempt: 1,
            engine: Engine::Bing,
            query: "test".into(),
            elapsed_ms: 0,
            result_count: 0,
            retries: 0,
            success: false,
            outcome: "cancelled_min_results".into(),
            error: None,
            raw_result_count: 0,
            filtered_count: 0,
        }]));
        let recorder = Arc::new(Mutex::new(Recorder::new()));
        let task_recorder = Arc::clone(&recorder);
        let task_diagnostics = Arc::clone(&diagnostics);
        let mut job = Box::pin(async move {
            let _timer = DiagnosticTimer {
                engine: Engine::Bing,
                state: ProviderOutcome::CancelledCaller,
                states: None,
                telemetry: crate::telemetry::Span::new("kestrel.provider"),
                diagnostics: task_diagnostics,
                index: 0,
                started: Instant::now(),
                recorder: Arc::clone(&task_recorder),
                completed: false,
                deadline: false,
                fanout_cancelled: Some(Arc::new(AtomicU8::new(FANOUT_MIN_RESULTS))),
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
        assert_eq!(entries[0].outcome, "cancelled_min_results");
        let snapshot = recorder.lock().unwrap().finish(false);
        assert_eq!(snapshot.send_attempts, 2);
        assert_eq!(snapshot.cancellation_phase, Some(Phase::Backoff));
    }

    #[tokio::test]
    async fn deadline_includes_provider_queue_and_records_reason() {
        let _telemetry = crate::telemetry::test_export_guard();
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

    #[tokio::test]
    async fn fanout_deadline_retains_success_and_reports_total_failure() {
        let _telemetry = crate::telemetry::test_export_guard();
        use std::pin::Pin;

        type Job<'a> =
            Pin<Box<dyn Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)> + 'a>>;

        let clients = SearchClients::new(&[Engine::Bing]).unwrap();
        for has_success in [true, false] {
            let diagnostics = Arc::new(Mutex::new(Vec::new()));
            let pending = FuturesUnordered::<Job<'_>>::new();
            if has_success {
                pending.push(Box::pin(async {
                    (
                        0,
                        Ok(vec![SearchResult::parsed(
                            "fast result".into(),
                            "https://example.com/fast".into(),
                            String::new(),
                            "useful snippet".into(),
                        )]),
                    )
                }));
            }
            let entries = Arc::clone(&diagnostics);
            let clients = &clients;
            // A provider blocked on the queue never performs a network request.
            pending.push(Box::pin(async move {
                (
                    1,
                    run_one(
                        "query",
                        Engine::Bing,
                        clients,
                        Arc::new(Semaphore::new(0)),
                        entries,
                        "",
                        TimeFilter::Any,
                        Some(tokio::time::Instant::now() + Duration::from_millis(5)),
                        None,
                    )
                    .await,
                )
            }));
            let (outcomes, cancelled) =
                tokio::time::timeout(Duration::from_secs(1), collect_fanout(pending, 5))
                    .await
                    .expect("fanout must finish at its deadline");
            assert_eq!(cancelled, 0);
            let merged = merge_outcomes(outcomes);
            if has_success {
                let results = merged.unwrap();
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].title, "fast result");
            } else {
                assert!(
                    merged
                        .unwrap_err()
                        .to_string()
                        .contains("Every search failed")
                );
            }
            let entries = diagnostics.lock().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].outcome, "deadline");
            assert!(!entries[0].success);
        }
    }

    #[test]
    fn additional_html_parsers_reject_shells_and_accept_explicit_empty() {
        let _telemetry = crate::telemetry::test_export_guard();
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
        let _telemetry = crate::telemetry::test_export_guard();
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
mod bing_fidelity;

#[cfg(test)]
#[path = "search/diagnostic_tests.rs"]
mod diagnostic_tests;

#[cfg(test)]
mod min_results_tests;

#[cfg(test)]
pub(crate) async fn test_cancel_with_blocked_diagnostics(engine: Engine) {
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let result = run_one_job(
        "slow sink",
        engine,
        Arc::new(Semaphore::new(1)),
        Arc::clone(&diagnostics),
        Some(tokio::time::Instant::now() + Duration::from_millis(20)),
        None,
        std::future::pending(),
    );
    let result = tokio::time::timeout(Duration::from_secs(2), result)
        .await
        .unwrap();
    assert!(result.is_err());
    let entries = diagnostics.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, "deadline");
}
