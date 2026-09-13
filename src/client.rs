//! Reusable search and fetch clients for connection pooling across calls.

use std::time::Duration;

use futures_util::{StreamExt, stream};

use crate::cache::PageCache;
use crate::fetcher::{
    build_client_with_transport, fetch_all_reusing_client, fetch_all_reusing_client_with_budget,
    fetch_all_reusing_client_with_cache, fetch_all_reusing_client_with_diagnostics,
};
use crate::model::{Engine, FetchOptions, FetchReport, SearchOptions, SearchResult, TimeFilter};
use crate::numeric::before_deadline;
use crate::search::{
    KestrelError, SearchClients, search_many_reusing_clients, search_many_reusing_clients_detailed,
    search_with_clients,
};

/// A reusable Kestrel client that retains HTTP connection pools across calls.
#[derive(Clone)]
pub struct KestrelClient {
    pub(crate) search: SearchClients,
    pub(crate) fetch: reqwest::Client,
}

impl KestrelClient {
    /// Build clients for every supported provider and page fetching.
    pub fn new() -> Result<Self, KestrelError> {
        Self::with_transport(crate::TransportOptions::default())
    }

    /// Build retained pools with an explicit transport policy. Clones share the pools.
    pub fn with_transport(transport: crate::TransportOptions) -> Result<Self, KestrelError> {
        crate::telemetry::scope_sync("kestrel.initialize", || {
            transport.validate()?;
            Ok(Self {
                search: SearchClients::with_transport(
                    &[Engine::Duckduckgo, Engine::Bing, Engine::Yahoo],
                    &transport,
                )?,
                fetch: build_client_with_transport(&transport)?,
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
        fetch_all_reusing_client(urls, options, &self.fetch).await
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
        fetch_all_reusing_client_with_budget(urls, options, &self.fetch, Some(budget)).await
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
        fetch_all_reusing_client_with_diagnostics(urls, options, &self.fetch, budget).await
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
