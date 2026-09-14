//! Reusable search and fetch clients for connection pooling across calls.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{StreamExt, stream};

use crate::cache::PageCache;
use crate::fetcher::{
    ParserPool, build_client_with_transport, fetch_all_reusing_client_with_cache,
    fetch_all_with_parser_pool,
};
use crate::model::{Engine, FetchOptions, FetchReport, SearchOptions, SearchResult, TimeFilter};
use crate::numeric::before_deadline;
use crate::search::{
    KestrelError, SearchClients, search_many_reusing_clients, search_many_reusing_clients_detailed,
    search_with_clients,
};

/// A reusable client retaining HTTP pools and shared page parser capacity.
/// Default aggregate page capacity is 10, shared across calls and clones. A batch
/// obeys both this cap and its own `FetchOptions::parse_concurrency` limit.
/// Cancellation returns without waiting for blocking parsers; their capacity
/// remains occupied until they exit. Runtime shutdown may still wait for them.
///
/// Provider parsing has a separate fixed aggregate limit of ten queued/running
/// blocking workers shared across this client and its clones. Cancelled calls
/// retain worker capacity until the work exits. Per-call search concurrency
/// still limits requests.
#[derive(Clone)]
pub struct KestrelClient {
    pub(crate) search: SearchClients,
    pub(crate) fetch: reqwest::Client,
    recovery: Option<crate::SearchRecovery>,
    pub(crate) parsing: Arc<ParserPool>,
}

impl KestrelClient {
    /// Build clients for every supported provider and page fetching.
    pub fn new() -> Result<Self, KestrelError> {
        Self::with_transport(crate::TransportOptions::default())
    }

    /// Build retained pools with an explicit transport policy. Clones share the pools.
    pub fn with_transport(transport: crate::TransportOptions) -> Result<Self, KestrelError> {
        Self::with_transport_and_parser_capacity(
            transport,
            FetchOptions::default().parse_concurrency,
        )
    }

    /// Build a client with aggregate queued/running page parser capacity.
    /// Per-call `FetchOptions::parse_concurrency` additionally limits each batch.
    /// Clones share capacity; separate clients and free functions do not.
    pub fn with_parser_capacity(capacity: usize) -> Result<Self, KestrelError> {
        Self::with_transport_and_parser_capacity(crate::TransportOptions::default(), capacity)
    }

    /// Configure transport and aggregate parser capacity (1..=Semaphore::MAX_PERMITS).
    /// Capacity remains owned by blocking jobs after cancellation or budget expiry.
    pub fn with_transport_and_parser_capacity(
        transport: crate::TransportOptions,
        capacity: usize,
    ) -> Result<Self, KestrelError> {
        Self::build_for_engines(&[Engine::Yahoo], transport, capacity)
    }

    /// Build only the transports needed by `engines`, with shared page parser capacity.
    /// All non-Yahoo providers share one transport and remain available. Yahoo is
    /// available only when included here; requesting it otherwise returns
    /// `KestrelError::InvalidRequest` before starting a search. Clones retain this
    /// policy and share pools. Initialization completes before any search budget.
    /// Use `new` or `with_parser_capacity` for unrestricted reusable clients.
    pub fn with_engines_and_parser_capacity(
        engines: &[Engine],
        capacity: usize,
    ) -> Result<Self, KestrelError> {
        Self::build_for_engines(engines, crate::TransportOptions::default(), capacity)
    }

    fn build_for_engines(
        engines: &[Engine],
        transport: crate::TransportOptions,
        capacity: usize,
    ) -> Result<Self, KestrelError> {
        crate::numeric::concurrency("parser capacity", capacity)?;
        crate::telemetry::scope_sync("kestrel.initialize", || {
            transport.validate()?;
            Ok(Self {
                search: SearchClients::with_transport(engines, &transport)?,
                fetch: build_client_with_transport(&transport)?,
                recovery: None,
                parsing: Arc::new(ParserPool::new(capacity)),
            })
        })
    }

    /// Replay and record provider progress for multi-query searches. Page caching is independent.
    pub fn with_recovery(mut self, recovery: crate::SearchRecovery) -> Self {
        self.recovery = Some(recovery);
        self
    }

    /// Search one provider while retaining its connection pool for later calls.
    pub async fn search(
        &self,
        query: &str,
        engine: Engine,
        region: &str,
        time_filter: TimeFilter,
    ) -> Result<Vec<SearchResult>, KestrelError> {
        search_with_clients(query, engine, region, time_filter, &self.search).await
    }

    /// Search one or more query/provider combinations with reusable clients.
    pub async fn search_many(
        &self,
        queries: &[String],
        options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, KestrelError> {
        search_many_reusing_clients(queries, options, &self.search, self.recovery.as_ref()).await
    }

    /// Search one or more query/provider combinations with provider diagnostics.
    pub async fn search_many_detailed(
        &self,
        queries: &[String],
        options: &SearchOptions,
    ) -> Result<crate::model::SearchReport, KestrelError> {
        search_many_reusing_clients_detailed(queries, options, &self.search, self.recovery.as_ref())
            .await
    }

    /// Fetch and extract pages while retaining connections for later calls.
    pub async fn fetch_all(
        &self,
        urls: &[String],
        options: &FetchOptions,
    ) -> Result<Vec<Option<String>>, KestrelError> {
        Ok(self.fetch_all_detailed(urls, options, None).await?.contents)
    }

    /// Fetch pages up to a total budget, retaining every result completed in time.
    pub async fn fetch_all_with_budget(
        &self,
        urls: &[String],
        options: &FetchOptions,
        budget: Duration,
    ) -> Result<Vec<Option<String>>, KestrelError> {
        if budget.is_zero() {
            return Err(KestrelError::InvalidRequest(
                "fetch budget must be greater than zero".into(),
            ));
        }
        Ok(self
            .fetch_all_detailed(urls, options, Some(budget))
            .await?
            .contents)
    }

    /// Fetch pages and return phase-level diagnostics, with an optional deadline.
    pub async fn fetch_all_detailed(
        &self,
        urls: &[String],
        options: &FetchOptions,
        budget: Option<Duration>,
    ) -> Result<FetchReport, KestrelError> {
        if budget.is_some_and(|duration| duration.is_zero()) {
            return Err(KestrelError::InvalidRequest(
                "fetch budget must be greater than zero".into(),
            ));
        }
        fetch_all_with_parser_pool(urls, options, &self.fetch, budget, Some(&self.parsing)).await
    }

    /// Use fresh cached text first, fetching only misses within an optional budget.
    pub async fn fetch_all_cached(
        &self,
        urls: &[String],
        options: &FetchOptions,
        cache: &PageCache,
        budget: Option<Duration>,
    ) -> Result<Vec<Option<String>>, KestrelError> {
        Ok(self
            .fetch_all_cached_detailed(urls, options, cache, budget)
            .await?
            .contents)
    }

    /// Use cached text first and return cache plus phase-level fetch diagnostics.
    pub async fn fetch_all_cached_detailed(
        &self,
        urls: &[String],
        options: &FetchOptions,
        cache: &PageCache,
        budget: Option<Duration>,
    ) -> Result<FetchReport, KestrelError> {
        crate::telemetry::scope_result("kestrel.cache_fetch", async {
            let deadline = budget
                .map(|value| crate::numeric::deadline("fetch budget", value))
                .transpose()?;
            crate::fetcher::validate_options(options)?;
            let cache = &cache.for_response_limit(options.max_response_bytes);
            let mut results = vec![None; urls.len()];
            let mut hit_indices = Vec::new();
            let mut reads = stream::iter(0..urls.len())
                .map(|index| async move {
                    (index, cache.get(&urls[index], options.content_limit).await)
                })
                .buffer_unordered(crate::cache::CACHE_IO_CONCURRENCY);
            let mut storage_exhausted = false;
            for _ in 0..urls.len() {
                match before_deadline(deadline, reads.next()).await {
                    Ok(Some((index, content))) => {
                        if content.is_some() {
                            hit_indices.push(index);
                        }
                        results[index] = content;
                    }
                    Ok(None) => break,
                    Err(()) => {
                        storage_exhausted = true;
                        break;
                    }
                }
            }
            drop(reads);
            crate::telemetry::payload("cache.contents", &results);
            let missing_indices: Vec<_> = results
                .iter()
                .enumerate()
                .filter_map(|(index, content)| content.is_none().then_some(index))
                .collect();
            let missing_urls: Vec<_> = missing_indices
                .iter()
                .map(|&index| urls[index].clone())
                .collect();
            let mut report = if missing_urls.is_empty() {
                FetchReport {
                    contents: Vec::new(),
                    pages: Vec::new(),
                    budget_exhausted: false,
                    cancelled: 0,
                    cache_hits: 0,
                    cache_misses: 0,
                }
            } else if storage_exhausted
                || deadline.is_some_and(|end| tokio::time::Instant::now() >= end)
            {
                FetchReport {
                    contents: vec![None; missing_urls.len()],
                    pages: Vec::new(),
                    budget_exhausted: true,
                    cancelled: missing_urls.len(),
                    cache_hits: 0,
                    cache_misses: missing_urls.len(),
                }
            } else {
                Box::pin(fetch_all_reusing_client_with_cache(
                    &missing_urls,
                    options,
                    &self.fetch,
                    deadline,
                    Some(cache),
                    self.recovery.as_ref(),
                    Some(&self.parsing),
                ))
                .await?
            };
            // Preserve all completed output, including results retained at a storage deadline.
            for (&index, content) in missing_indices
                .iter()
                .zip(std::mem::take(&mut report.contents))
            {
                results[index] = content;
            }
            report.pages.extend(
                hit_indices.iter().map(|&index| {
                    crate::model::PageFetchDiagnostic::cache_hit(urls[index].clone())
                }),
            );
            report.contents = results;
            report.budget_exhausted |= storage_exhausted;
            if storage_exhausted {
                eprintln!(
                    "[kestrel] Cache work reached the fetch deadline; retained completed results."
                );
            }
            crate::telemetry::attribute("kestrel.cache_hits", hit_indices.len() as i64);
            crate::telemetry::payload("cache_fetch.output", &report.contents);
            report.cache_hits = hit_indices.len();
            report.cache_misses = missing_urls.len();
            Ok(report)
        })
        .await
    }
}

#[cfg(test)]
mod construction_tests {
    use super::*;

    #[test]
    fn scoped_clients_only_construct_yahoo_when_selected() {
        let _telemetry = crate::telemetry::test_export_guard();
        for engines in [
            vec![],
            vec![Engine::Bing],
            vec![Engine::Qwant, Engine::Mojeek],
        ] {
            let client = KestrelClient::with_engines_and_parser_capacity(&engines, 2).unwrap();
            assert!(client.search.yahoo.is_none());
            assert!(client.clone().search.yahoo.is_none());
        }
        let client = KestrelClient::with_engines_and_parser_capacity(&[Engine::Yahoo], 2).unwrap();
        assert!(client.search.yahoo.is_some());
        assert!(client.clone().search.yahoo.is_some());
        assert!(KestrelClient::with_engines_and_parser_capacity(&[Engine::Bing], 0).is_err());
    }

    #[test]
    fn existing_constructors_keep_all_provider_transports() {
        let _telemetry = crate::telemetry::test_export_guard();
        for client in [
            KestrelClient::new(),
            KestrelClient::with_parser_capacity(2),
            KestrelClient::with_transport(crate::TransportOptions::default()),
            KestrelClient::with_transport_and_parser_capacity(
                crate::TransportOptions::default(),
                2,
            ),
        ] {
            assert!(client.unwrap().search.yahoo.is_some());
        }
    }

    #[tokio::test]
    async fn scoped_client_rejects_uninitialized_yahoo_before_any_requests() {
        let _telemetry = crate::telemetry::test_export_guard();
        let client = KestrelClient::with_engines_and_parser_capacity(&[Engine::Bing], 2).unwrap();
        for client in [client.clone(), client] {
            assert!(matches!(
                client
                    .search("test", Engine::Yahoo, "wt-wt", TimeFilter::Any)
                    .await,
                Err(KestrelError::InvalidRequest(_))
            ));
            let queries = vec!["test".to_owned()];
            let options = SearchOptions {
                engines: vec![Engine::Bing, Engine::Yahoo],
                ..Default::default()
            };
            assert!(matches!(
                client.search_many(&queries, &options).await,
                Err(KestrelError::InvalidRequest(_))
            ));
            assert!(matches!(
                client.search_many_detailed(&queries, &options).await,
                Err(KestrelError::InvalidRequest(_))
            ));
        }
    }
}
