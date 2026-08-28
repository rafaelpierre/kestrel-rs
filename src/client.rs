//! Reusable search and fetch clients for connection pooling across calls.

use std::time::Duration;

use futures_util::future::join_all;

use crate::cache::PageCache;
use crate::fetcher::{
    build_client, fetch_all_reusing_client, fetch_all_reusing_client_with_budget,
    fetch_all_reusing_client_with_diagnostics,
};
use crate::model::{Engine, FetchOptions, FetchReport, SearchOptions, SearchResult, TimeFilter};
use crate::search::{
    KestrelError, SearchClients, search_many_reusing_clients, search_many_reusing_clients_detailed,
    search_with_clients,
};

/// A reusable Kestrel client that retains HTTP connection pools across calls.
pub struct KestrelClient {
    search: SearchClients,
    fetch: reqwest::Client,
}

impl KestrelClient {
    /// Build clients for every supported provider and page fetching.
    pub fn new() -> Result<Self, KestrelError> {
        Ok(Self {
            search: SearchClients::new(&[Engine::Duckduckgo, Engine::Bing, Engine::Yahoo])?,
            fetch: build_client()?,
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
        if budget.is_some_and(|duration| duration.is_zero()) {
            return Err(KestrelError::InvalidRequest(
                "fetch budget must be greater than zero".into(),
            ));
        }
        let cached = join_all(urls.iter().map(|url| cache.get(url, options.content_limit))).await;
        let mut results = cached;
        let misses: Vec<(usize, String)> = results
            .iter()
            .enumerate()
            .filter(|(_, content)| content.is_none())
            .map(|(index, _)| (index, urls[index].clone()))
            .collect();
        let missing_urls: Vec<String> = misses.iter().map(|(_, url)| url.clone()).collect();
        let fetched =
            fetch_all_reusing_client_with_budget(&missing_urls, options, &self.fetch, budget)
                .await?;
        for ((index, url), content) in misses.into_iter().zip(fetched) {
            if let Some(content) = content {
                let _ = cache.put(&url, options.content_limit, &content).await;
                results[index] = Some(content);
            }
        }
        let _ = cache.prune().await;
        Ok(results)
    }

    /// Use cached text first and return cache plus phase-level fetch diagnostics.
    pub async fn fetch_all_cached_detailed(
        &self,
        urls: &[String],
        options: &FetchOptions,
        cache: &PageCache,
        budget: Option<Duration>,
    ) -> Result<FetchReport, KestrelError> {
        if budget.is_some_and(|duration| duration.is_zero()) {
            return Err(KestrelError::InvalidRequest(
                "fetch budget must be greater than zero".into(),
            ));
        }
        let cached = join_all(urls.iter().map(|url| cache.get(url, options.content_limit))).await;
        let cache_hits = cached.iter().filter(|content| content.is_some()).count();
        let mut results = cached;
        let misses: Vec<(usize, String)> = results
            .iter()
            .enumerate()
            .filter(|(_, content)| content.is_none())
            .map(|(index, _)| (index, urls[index].clone()))
            .collect();
        let missing_urls: Vec<String> = misses.iter().map(|(_, url)| url.clone()).collect();
        let mut report =
            fetch_all_reusing_client_with_diagnostics(&missing_urls, options, &self.fetch, budget)
                .await?;
        for ((index, url), content) in misses.into_iter().zip(report.contents) {
            if let Some(content) = content {
                let _ = cache.put(&url, options.content_limit, &content).await;
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
        report.cache_hits = cache_hits;
        report.cache_misses = missing_urls.len();
        let _ = cache.prune().await;
        Ok(report)
    }
}
