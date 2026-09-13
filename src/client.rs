//! Reusable search and fetch clients for connection pooling across calls.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;

use crate::cache::PageCache;
use crate::fetcher::{ParserPool, build_client_with_transport, fetch_all_with_parser_pool};
use crate::model::{Engine, FetchOptions, FetchReport, SearchOptions, SearchResult, TimeFilter};
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
        crate::numeric::concurrency("parser capacity", capacity)?;
        crate::telemetry::scope_sync("kestrel.initialize", || {
            transport.validate()?;
            Ok(Self {
                search: SearchClients::with_transport(
                    &[Engine::Duckduckgo, Engine::Bing, Engine::Yahoo],
                    &transport,
                )?,
                fetch: build_client_with_transport(&transport)?,
                parsing: Arc::new(ParserPool::new(capacity)),
            })
        })
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
        search_many_reusing_clients(queries, options, &self.search).await
    }

    /// Search one or more query/provider combinations with provider diagnostics.
    pub async fn search_many_detailed(
        &self,
        queries: &[String],
        options: &SearchOptions,
    ) -> Result<crate::model::SearchReport, KestrelError> {
        search_many_reusing_clients_detailed(queries, options, &self.search).await
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
            if budget.is_some_and(|duration| duration.is_zero()) {
                return Err(KestrelError::InvalidRequest(
                    "fetch budget must be greater than zero".into(),
                ));
            }
            crate::fetcher::validate_options(options)?;
            if let Some(budget) = budget {
                crate::numeric::duration("fetch budget", budget)?;
            }
            let cached =
                join_all(urls.iter().map(|url| cache.get(url, options.content_limit))).await;
            crate::telemetry::payload("cache.contents", &cached);
            let cache_hits = cached.iter().filter(|content| content.is_some()).count();
            let mut results = cached;
            let misses: Vec<(usize, String)> = results
                .iter()
                .enumerate()
                .filter(|(_, content)| content.is_none())
                .map(|(index, _)| (index, urls[index].clone()))
                .collect();
            let missing_urls: Vec<String> = misses.iter().map(|(_, url)| url.clone()).collect();
            let mut report = fetch_all_with_parser_pool(
                &missing_urls,
                options,
                &self.fetch,
                budget,
                Some(&self.parsing),
            )
            .await?;
            let capped_urls: HashSet<&str> = report
                .pages
                .iter()
                .filter(|page| page.response_bytes >= options.max_response_bytes)
                .map(|page| page.url.as_str())
                .collect();
            for ((index, url), content) in misses.into_iter().zip(report.contents) {
                if let Some(content) = content {
                    // A cap-sized body may be partial, even if it ended exactly at
                    // the cap. Do not let it satisfy a later, larger byte budget.
                    if !capped_urls.contains(url.as_str()) {
                        let _ = cache.put(&url, options.content_limit, &content).await;
                    }
                    results[index] = Some(content);
                }
            }
            report.pages.extend(
                urls.iter()
                    .zip(&results)
                    .filter(|(url, _)| !missing_urls.contains(url))
                    .map(|(url, _)| crate::model::PageFetchDiagnostic::cache_hit(url.clone())),
            );
            report.contents = results;
            crate::telemetry::attribute("kestrel.cache_hits", cache_hits as i64);
            crate::telemetry::payload("cache_fetch.output", &report.contents);
            report.cache_hits = cache_hits;
            report.cache_misses = missing_urls.len();
            let _ = cache.prune().await;
            Ok(report)
        })
        .await
    }
}
