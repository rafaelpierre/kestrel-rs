//! Multi-provider search, retry, normalization, and fair merging.

use std::collections::{HashMap, HashSet};
use std::future::Future;
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

const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
const SEARCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Error)]
pub enum KestrelError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Search(String),
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

pub(crate) struct SearchClients {
    standard: reqwest::Client,
    yahoo: Option<primp::Client>,
}

impl SearchClients {
    pub(crate) fn new(engines: &[Engine]) -> Result<Self, KestrelError> {
        let standard = reqwest::Client::builder()
            .user_agent(SEARCH_USER_AGENT)
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::ACCEPT_LANGUAGE,
                    reqwest::header::HeaderValue::from_static("en-US,en;q=0.9"),
                );
                headers
            })
            .timeout(SEARCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        let yahoo = engines.contains(&Engine::Yahoo).then(|| {
            primp::Client::builder()
                .impersonate(primp::Impersonate::ChromeV146)
                .timeout(SEARCH_TIMEOUT)
                .build()
        });
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
    run_provider(query, engine, region, time_filter, clients)
        .await
        .map(|response| response.results)
}

struct ProviderResponse {
    results: Vec<SearchResult>,
    retries: usize,
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
    let semaphore = Arc::new(Semaphore::new(options.max_concurrency));
    let diagnostics = Arc::new(Mutex::new(Vec::new()));

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
async fn run_fanout_query(
    query: &str,
    engines: &[Engine],
    clients: &SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    region: &str,
    time_filter: TimeFilter,
    provider_quorum: Option<usize>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize) {
    let pending = FuturesUnordered::new();
    for (index, engine) in engines.iter().copied().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let diagnostics = Arc::clone(&diagnostics);
        pending.push(async move {
            (
                index,
                run_one(
                    query,
                    engine,
                    clients,
                    semaphore,
                    diagnostics,
                    region,
                    time_filter,
                )
                .await,
            )
        });
    }
    collect_fanout(pending, provider_quorum).await
}

async fn collect_fanout<F>(
    mut pending: FuturesUnordered<F>,
    provider_quorum: Option<usize>,
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

async fn run_one(
    query: &str,
    engine: Engine,
    clients: &SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    region: &str,
    time_filter: TimeFilter,
) -> Result<Vec<SearchResult>, KestrelError> {
    let _permit = semaphore.acquire().await.expect("semaphore remains open");
    let started = Instant::now();
    let outcome = run_provider(query, engine, region, time_filter, clients).await;
    diagnostics
        .lock()
        .expect("search diagnostic lock is not poisoned")
        .push(ProviderSearchDiagnostic {
            engine,
            query: query.to_owned(),
            elapsed_ms: elapsed_millis(started),
            result_count: outcome
                .as_ref()
                .map_or(0, |response| response.results.len()),
            retries: outcome.as_ref().map_or(0, |response| response.retries),
            success: outcome.is_ok(),
        });
    outcome.map(|response| with_provenance(response.results, engine, query))
}

async fn run_with_fallback(
    query: &str,
    engines: &[Engine],
    clients: &SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    region: &str,
    time_filter: TimeFilter,
) -> Result<Vec<SearchResult>, KestrelError> {
    let mut errors = Vec::new();
    for (index, engine) in engines.iter().enumerate() {
        match run_one(
            query,
            *engine,
            clients,
            Arc::clone(&semaphore),
            Arc::clone(&diagnostics),
            region,
            time_filter,
        )
        .await
        {
            Ok(results) => return Ok(results),
            Err(error) => {
                errors.push(error.to_string());
                crate::log_event!(
                    "search_fallback",
                    "query" => query,
                    "failed_engine" => engine.as_str(),
                    "next_engine" => engines.get(index + 1).map_or("", |next| next.as_str()),
                );
            }
        }
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
    let result = match engine {
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
    };
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
        results: parse_duckduckgo_results(&text),
        retries,
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
        results: parse_bing_results(&text),
        retries,
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
    let mut last_error = None;
    for attempt in 1..=3 {
        match client
            .get("https://search.yahoo.com/search")
            .query(&params)
            .timeout(SEARCH_TIMEOUT)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                return Ok(ProviderResponse {
                    results: parse_yahoo_results(&response.text().await?),
                    retries: attempt - 1,
                });
            }
            Ok(response) => {
                let status = response.status().as_u16();
                let retryable = status == 408 || status == 429 || status >= 500;
                let error = KestrelError::Search(format!("Yahoo returned HTTP {status}"));
                if !retryable || attempt == 3 {
                    return Err(error);
                }
                last_error = Some(error);
            }
            Err(error) => {
                if attempt == 3 {
                    return Err(error.into());
                }
                last_error = Some(error.into());
            }
        }
        log_retry(Engine::Yahoo, query, attempt, last_error.as_ref());
        retry_delay(attempt).await;
    }
    Err(last_error.unwrap_or_else(|| KestrelError::Search("Yahoo request failed".into())))
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
        match build().send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok((response.text().await?, attempt - 1));
                }
                let retryable =
                    status.as_u16() == 408 || status.as_u16() == 429 || status.is_server_error();
                let error = response.error_for_status().expect_err("non-success status");
                if !retryable || attempt == 3 {
                    return Err(error.into());
                }
                last_error = Some(error.into());
            }
            Err(error) => {
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

async fn retry_delay(attempt: usize) {
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
            let rank = index + 1;
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

    const DDG: &str = r#"
    <div class="result results_links results_links_deep web-result">
      <h2 class="result__title"><a class="result__a" href="https://example.com">Example</a></h2>
      <a class="result__url">example.com</a><a class="result__snippet">A useful result</a>
    </div><div class="result results_links results_links_deep web-result"><h2></h2></div>"#;

    #[test]
    fn parses_duckduckgo() {
        let results = parse_duckduckgo_results(DDG);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example");
        assert_eq!(results[0].snippet, "A useful result");
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
}
