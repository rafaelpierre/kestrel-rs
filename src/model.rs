use std::time::Duration;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// A supported HTML search provider.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum Engine {
    #[default]
    Duckduckgo,
    Bing,
    Yahoo,
    Dogpile,
    Ecosia,
    Swisscows,
    Yep,
    Qwant,
    Mojeek,
}

impl Engine {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duckduckgo => "duckduckgo",
            Self::Bing => "bing",
            Self::Yahoo => "yahoo",
            Self::Dogpile => "dogpile",
            Self::Ecosia => "ecosia",
            Self::Swisscows => "swisscows",
            Self::Yep => "yep",
            Self::Qwant => "qwant",
            Self::Mojeek => "mojeek",
        }
    }
}

impl std::fmt::Display for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Concurrent provider orchestration. Retained for API compatibility; fanout is the only mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum SearchMode {
    #[default]
    Fanout,
}

impl std::fmt::Display for SearchMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Fanout => "fanout",
        })
    }
}

/// Provider recency filter.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum TimeFilter {
    #[default]
    Any,
    D,
    W,
    M,
    Y,
}

impl TimeFilter {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::D => "d",
            Self::W => "w",
            Self::M => "m",
            Self::Y => "y",
        }
    }
}

/// One provider/query occurrence of a deduplicated result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceOccurrence {
    pub engine: Engine,
    pub query: String,
    pub rank: usize,
}

/// Normalized result returned by the library and CLI.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub display_url: String,
    pub snippet: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_rank: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceOccurrence>,
}

impl SearchResult {
    pub(crate) fn parsed(title: String, url: String, display_url: String, snippet: String) -> Self {
        Self {
            title: title.trim().to_owned(),
            url: url.trim().to_owned(),
            display_url: display_url.trim().to_owned(),
            snippet: snippet.trim().to_owned(),
            content: None,
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: Vec::new(),
        }
    }
}

/// Controls provider orchestration.
#[derive(Clone, Debug)]
pub struct SearchOptions {
    /// Portable title/snippet constraints, or provider-native passthrough.
    pub query_syntax: crate::QuerySyntax,
    /// Providers searched concurrently; defaults to all nine supported engines in declaration order.
    pub engines: Vec<Engine>,
    pub mode: SearchMode,
    pub region: String,
    pub time_filter: TimeFilter,
    pub max_concurrency: usize,
    /// Stop fanout after this many providers per query return non-empty results.
    pub provider_quorum: Option<usize>,
    /// Total search budget including queueing and retries; disabled by default.
    pub search_budget: Option<Duration>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query_syntax: crate::QuerySyntax::default(),
            engines: Engine::value_variants().to_vec(),
            mode: SearchMode::Fanout,
            region: String::new(),
            time_filter: TimeFilter::Any,
            max_concurrency: 10,
            provider_quorum: None,
            search_budget: None,
        }
    }
}

/// Controls bounded page retrieval and extraction.
///
/// Per fetch batch, at most `max_concurrency` bodies are downloading or waiting
/// for parsing, plus `parse_concurrency` bodies owned by parsers. Each body is
/// limited to `max_response_bytes`. Decoded text, DOM expansion, HTTP transport
/// buffers, and retained output are additional memory. Download slots remain
/// occupied while waiting for a parser, applying backpressure to the network.
#[derive(Clone, Debug)]
pub struct FetchOptions {
    pub timeout: Duration,
    pub content_limit: usize,
    pub max_concurrency: usize,
    pub parse_concurrency: usize,
    /// Stop at this many decoded body bytes and extract the retained prefix.
    pub max_response_bytes: usize,
}

/// Outcome of retrieving and extracting one candidate page.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchOutcome {
    Success,
    NoContent,
    UnsupportedContentType,
    /// Legacy outcome retained for compatibility; page byte caps now extract a prefix.
    ResponseTooLarge,
    RequestFailed,
    CacheHit,
}

/// Phase timings and transfer metadata for one candidate page.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageFetchDiagnostic {
    pub url: String,
    pub outcome: FetchOutcome,
    pub queue_ms: u64,
    /// Send until response headers, including internal queueing and cold setup.
    pub request_ms: u64,
    pub download_ms: u64,
    pub parse_queue_ms: u64,
    pub parse_ms: u64,
    pub total_ms: u64,
    /// Retained decoded body bytes. Reaching the byte cap means content may be partial.
    pub response_bytes: usize,
    /// Negotiated protocol; absent for cache hits or failures before headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_version: Option<String>,
}

impl PageFetchDiagnostic {
    pub(crate) fn cache_hit(url: String) -> Self {
        Self {
            url,
            outcome: FetchOutcome::CacheHit,
            http_version: None,
            queue_ms: 0,
            request_ms: 0,
            download_ms: 0,
            parse_queue_ms: 0,
            parse_ms: 0,
            total_ms: 0,
            response_bytes: 0,
        }
    }
}

/// Contents plus diagnostics from one bounded fetch operation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FetchReport {
    pub contents: Vec<Option<String>>,
    pub pages: Vec<PageFetchDiagnostic>,
    pub budget_exhausted: bool,
    pub cancelled: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

/// Timing and outcome for one provider/query request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProviderSearchDiagnostic {
    pub engine: Engine,
    pub query: String,
    pub elapsed_ms: u64,
    pub result_count: usize,
    pub retries: usize,
    pub success: bool,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub raw_result_count: usize,
    #[serde(default)]
    pub filtered_count: usize,
}

/// Search results plus the provider requests that produced them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SearchReport {
    pub results: Vec<SearchResult>,
    pub providers: Vec<ProviderSearchDiagnostic>,
    /// Provider/query searches dropped after a fanout quorum was reached.
    pub cancelled: usize,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            content_limit: 2_000,
            max_concurrency: 10,
            parse_concurrency: 10,
            max_response_bytes: crate::fetcher::DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}
