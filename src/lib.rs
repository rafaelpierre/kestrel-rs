//! Kestrel Search: keyless multi-engine web search, extraction, and ranking.

pub mod benchmarking;
pub mod cache;
pub mod client;
pub mod config;
pub mod content_quality;
pub mod diagnostic_sink;
mod error;
pub mod fetcher;
mod http_client;
pub mod logging;
pub mod model;
mod numeric;
mod provider_diagnostics;
mod provider_outcome;
pub use provider_outcome::ProviderOutcome;
pub mod providers;
pub mod proxy;
pub mod ranking;
pub mod recovery;
pub use recovery::SearchRecovery;
pub mod search;
pub mod skill;
pub mod telemetry;
pub mod transport;
mod warmup;
pub use transport::TransportOptions;
pub use warmup::WarmupResult;

pub use cache::PageCache;
pub use client::KestrelClient;
pub use content_quality::{
    ContentQuality, ContentQualityReason, ContentQualityState, assess_content_quality,
};
pub use error::KestrelError;
pub use fetcher::{fetch_all, fetch_all_detailed};
pub use model::{
    Engine, FetchOptions, FetchOutcome, FetchReport, PageFetchDiagnostic, ProviderSearchDiagnostic,
    SearchMode, SearchOptions, SearchReport, SearchResult, SourceOccurrence, TimeFilter,
};
pub use ranking::{pre_rank_candidates, rank_results, rank_results_by_query};
pub use search::{search, search_blocking, search_many, search_many_detailed};
