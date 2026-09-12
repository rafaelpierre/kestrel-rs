//! Kestrel Search: keyless multi-engine web search, extraction, and ranking.

pub mod benchmarking;
pub mod cache;
pub mod client;
pub mod config;
pub mod fetcher;
mod http_client;
pub mod logging;
pub mod model;
mod provider_diagnostics;
pub mod providers;
mod query;
pub mod ranking;
pub use query::QuerySyntax;
pub mod search;
pub mod skill;
pub mod transport;
mod warmup;
pub use transport::TransportOptions;
pub use warmup::WarmupResult;

pub use cache::PageCache;
pub use client::KestrelClient;
pub use fetcher::{fetch_all, fetch_all_detailed};
pub use model::{
    Engine, FetchOptions, FetchOutcome, FetchReport, PageFetchDiagnostic, ProviderSearchDiagnostic,
    SearchMode, SearchOptions, SearchReport, SearchResult, SourceOccurrence, TimeFilter,
};
pub use ranking::{pre_rank_candidates, rank_results, rank_results_by_query};
pub use search::{KestrelError, search, search_blocking, search_many, search_many_detailed};
