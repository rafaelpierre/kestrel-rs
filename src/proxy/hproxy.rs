//! Bounded, shared HProxy discovery, independent of managed proxy routing.
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use serde::Deserialize;
use tokio::{
    sync::{Mutex, OnceCell, Semaphore},
    time::{Instant, sleep_until, timeout_at},
};
use url::Url;

/// HProxy destination capabilities. Both use a cleartext HTTP connection to the
/// proxy; `Https` denotes CONNECT, not TLS to the proxy itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Http,
    Https,
}

impl Protocol {
    fn label(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}
impl FromStr for Protocol {
    type Err = DiscoveryError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            _ => Err(DiscoveryError::UnsupportedProtocol),
        }
    }
}

/// Filtering is applied remotely and checked locally. Missing enrichment fails
/// an active filter, but is accepted when that filter is absent.
#[derive(Clone, Debug)]
pub struct DiscoveryOptions {
    pub protocols: Vec<Protocol>,
    pub country: Option<String>,
    pub min_uptime_pct: Option<f64>,
    pub max_latency_ms: Option<u32>,
    /// False requests and retains strictly alive rows only.
    pub include_recent: bool,
    /// Aggregate decoded bytes across all successful pages; overflow is an error.
    pub max_response_bytes: usize,
    /// Maximum distinct eligible endpoints; overflow is an error, never truncation.
    pub max_entries: usize,
    /// None is the ordinary single full-list request (no limit or offset).
    pub page_size: Option<usize>,
    pub max_pages: usize,
    /// Total HTTP attempts, including pages and retries, for this initialization.
    pub max_attempts: usize,
    pub deadline: Duration,
    pub request_timeout: Duration,
    /// At least 500ms: no burst allowance is used (at most 120 attempts/minute).
    pub min_attempt_interval: Duration,
}
impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            protocols: vec![Protocol::Http, Protocol::Https],
            country: None,
            min_uptime_pct: None,
            max_latency_ms: None,
            include_recent: false,
            max_response_bytes: 16 * 1024 * 1024,
            max_entries: 50_000,
            page_size: None,
            max_pages: 1,
            max_attempts: 3,
            deadline: Duration::from_secs(30),
            request_timeout: Duration::from_secs(10),
            min_attempt_interval: Duration::from_millis(500),
        }
    }
}
impl DiscoveryOptions {
    fn validate(&self) -> Result<(), DiscoveryError> {
        if self.protocols.is_empty()
            || self.max_response_bytes == 0
            || self.max_entries == 0
            || self.max_pages == 0
            || self.max_attempts == 0
            || self.page_size.is_some_and(|n| !(1..=10_000).contains(&n))
            || (self.page_size.is_none() && self.max_pages != 1)
            || self
                .country
                .as_ref()
                .is_some_and(|s| s.len() != 2 || !s.bytes().all(|b| b.is_ascii_alphabetic()))
            || self
                .min_uptime_pct
                .is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v))
            || self
                .max_latency_ms
                .is_some_and(|v| !(1..=120_000).contains(&v))
            || self.min_attempt_interval < Duration::from_millis(500)
        {
            return Err(DiscoveryError::InvalidOptions);
        }
        for duration in [
            self.deadline,
            self.request_timeout,
            self.min_attempt_interval,
        ] {
            if duration.is_zero() || Instant::now().checked_add(duration).is_none() {
                return Err(DiscoveryError::InvalidOptions);
            }
        }
        Ok(())
    }
}

/// Validated IP endpoint with advertised, supported destination capabilities.
#[derive(Clone, Debug)]
pub struct ProxyEndpoint {
    pub address: SocketAddr,
    pub protocols: Vec<Protocol>,
    pub country_code: Option<String>,
    pub latency_ms: Option<f64>,
    pub uptime_pct: Option<f64>,
}
impl ProxyEndpoint {
    /// Suitable for both reqwest::Proxy and primp::Proxy. Capabilities still need
    /// to be checked against the destination; discovery does not test health.
    pub fn proxy_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

/// Aggregate counts contain no raw response bodies or credentials.
#[derive(Clone, Debug, Default)]
pub struct DiscoveryReport {
    pub endpoints: Vec<ProxyEndpoint>,
    pub malformed_rows: usize,
    pub unsupported_rows: usize,
    pub filtered_rows: usize,
    pub duplicate_rows: usize,
    pub pages: usize,
    pub attempts: usize,
    pub decoded_bytes: usize,
}

/// Discovery failures never yield a partial list. Shared clones cache failures
/// as well as successes; construct a new adapter to start another initialization.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("invalid HProxy discovery options or endpoint")]
    InvalidOptions,
    #[error(
        "unsupported proxy protocol; only http and https (CONNECT) are supported; SOCKS is unsupported"
    )]
    UnsupportedProtocol,
    #[error("HProxy bootstrap transport failed")]
    Transport,
    #[error("HProxy discovery deadline exceeded")]
    Deadline,
    #[error("HProxy discovery attempt limit exceeded")]
    Attempts,
    #[error("HProxy discovery HTTP {0}")]
    Http(u16),
    #[error("HProxy Retry-After is invalid or exceeds the discovery deadline")]
    RetryAfter,
    #[error("HProxy response exceeds the aggregate decoded-byte limit")]
    ResponseTooLarge,
    #[error("HProxy list exceeds the retained endpoint limit")]
    TooManyEntries,
    #[error("HProxy JSON must be a complete array of records")]
    InvalidJson,
    #[error("HProxy pagination is incomplete or inconsistent")]
    IncompleteList,
    #[error("HProxy list contains no eligible HTTP/CONNECT endpoints")]
    Empty,
    #[error("HProxy parser task failed")]
    Parser,
}

struct State {
    end: Option<Instant>,
    next_attempt: Instant,
    attempts: usize,
}
impl State {
    async fn reserve_attempt(
        &mut self,
        options: &DiscoveryOptions,
        end: Instant,
    ) -> Result<(), DiscoveryError> {
        if self.attempts >= options.max_attempts {
            return Err(DiscoveryError::Attempts);
        }
        if self.next_attempt >= end {
            return Err(DiscoveryError::Deadline);
        }
        sleep_until(self.next_attempt).await;
        let now = Instant::now();
        if now >= end {
            return Err(DiscoveryError::Deadline);
        }
        self.attempts += 1;
        self.next_attempt = now
            .checked_add(options.min_attempt_interval)
            .ok_or(DiscoveryError::Deadline)?;
        Ok(())
    }
}

struct Inner {
    client: reqwest::Client,
    endpoint: Url,
    options: DiscoveryOptions,
    result: OnceCell<Result<Arc<DiscoveryReport>, DiscoveryError>>,
    state: Mutex<State>,
    parser: Arc<Semaphore>,
}

/// Clone to coalesce discovery, throttling and cached results. Construction is
/// synchronous and performs no lookup; call `discover` explicitly.
#[derive(Clone)]
pub struct HProxyDiscovery {
    inner: Arc<Inner>,
}
impl HProxyDiscovery {
    /// Separate bootstrap pool preserving environment/system proxy settings.
    pub fn new(options: DiscoveryOptions) -> Result<Self, DiscoveryError> {
        Self::with_builder(
            Url::parse("https://hproxy.com/api/proxy-list")
                .map_err(|_| DiscoveryError::InvalidOptions)?,
            crate::TransportOptions::default().standard_builder(),
            options,
        )
    }

    /// Inject an endpoint and bootstrap builder (for example explicit proxy
    /// settings or test trust roots). Redirects and automatic transport retries
    /// are disabled here so every HTTP request consumes a declared attempt.
    /// Endpoint credentials, query strings and fragments are rejected.
    pub fn with_builder(
        endpoint: Url,
        builder: reqwest::ClientBuilder,
        options: DiscoveryOptions,
    ) -> Result<Self, DiscoveryError> {
        options.validate()?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(DiscoveryError::InvalidOptions);
        }
        let client = builder
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(options.request_timeout)
            .build()
            .map_err(|_| DiscoveryError::Transport)?;
        Ok(Self {
            inner: Arc::new(Inner {
                client,
                endpoint,
                options,
                result: OnceCell::new(),
                state: Mutex::new(State {
                    end: None,
                    next_attempt: Instant::now(),
                    attempts: 0,
                }),
                parser: Arc::new(Semaphore::new(1)),
            }),
        })
    }

    /// The deadline starts at first discovery and includes throttling, retries,
    /// pages and parsing. Cancellation consumes already-started attempts and
    /// does not reset that deadline or bypass a received Retry-After.
    pub async fn discover(&self) -> Result<Arc<DiscoveryReport>, DiscoveryError> {
        self.inner
            .result
            .get_or_init(|| async {
                let mut state = self.inner.state.lock().await;
                let end = *state
                    .end
                    .get_or_insert_with(|| Instant::now() + self.inner.options.deadline);
                if Instant::now() >= end {
                    return Err(DiscoveryError::Deadline);
                }
                timeout_at(end, self.load(&mut state, end))
                    .await
                    .map_err(|_| DiscoveryError::Deadline)?
                    .map(Arc::new)
            })
            .await
            .clone()
    }

    async fn load(
        &self,
        state: &mut State,
        end: Instant,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        let options = &self.inner.options;
        let mut report = DiscoveryReport::default();
        let mut offset = 0usize;
        let mut expected_total = None;
        let mut indexes = HashMap::new();
        loop {
            let mut url = self.inner.endpoint.clone();
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair("format", "json")
                    .append_pair("sort", "uptime");
                query.append_pair(
                    "protocol",
                    &options
                        .protocols
                        .iter()
                        .map(|p| p.label())
                        .collect::<Vec<_>>()
                        .join(","),
                );
                if let Some(country) = &options.country {
                    query.append_pair("country", &country.to_ascii_uppercase());
                }
                if let Some(value) = options.min_uptime_pct {
                    query.append_pair("min_uptime_pct", &value.to_string());
                }
                if let Some(value) = options.max_latency_ms {
                    query.append_pair("max_latency_ms", &value.to_string());
                }
                if options.include_recent {
                    query.append_pair("recent", "true");
                }
                if let Some(size) = options.page_size {
                    query
                        .append_pair("limit", &size.to_string())
                        .append_pair("offset", &offset.to_string());
                }
            }
            let mut response = self.request(url, state, end).await?;
            let count = header_count(&response, "x-total-count")?;
            let total = header_count(&response, "x-total-available")?;
            let remaining = options.max_response_bytes - report.decoded_bytes;
            if response
                .content_length()
                .is_some_and(|n| n > remaining as u64)
            {
                return Err(DiscoveryError::ResponseTooLarge);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| DiscoveryError::Transport)?
            {
                if chunk.len() > remaining - bytes.len() {
                    return Err(DiscoveryError::ResponseTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            report.decoded_bytes += bytes.len();
            let permit = self
                .inner
                .parser
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| DiscoveryError::Parser)?;
            let parse_options = options.clone();
            // One bounded parser per adapter, including after cancellation.
            let parsed = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                parse_page(&bytes, &parse_options)
            })
            .await
            .map_err(|_| DiscoveryError::Parser)??;
            report.pages += 1;
            report.malformed_rows += parsed.malformed_rows;
            report.unsupported_rows += parsed.unsupported_rows;
            report.filtered_rows += parsed.filtered_rows;
            report.duplicate_rows += parsed.duplicate_rows;
            let rows = parsed.rows;
            for endpoint in parsed.endpoints {
                if let Some(&index) = indexes.get(&endpoint.address) {
                    let existing: &mut ProxyEndpoint = &mut report.endpoints[index];
                    for protocol in endpoint.protocols {
                        if !existing.protocols.contains(&protocol) {
                            existing.protocols.push(protocol);
                        }
                    }
                    report.duplicate_rows += 1;
                } else {
                    if report.endpoints.len() == options.max_entries {
                        return Err(DiscoveryError::TooManyEntries);
                    }
                    indexes.insert(endpoint.address, report.endpoints.len());
                    report.endpoints.push(endpoint);
                }
            }
            if count.is_some_and(|count| count != rows) {
                return Err(DiscoveryError::IncompleteList);
            }
            offset = offset
                .checked_add(rows)
                .ok_or(DiscoveryError::IncompleteList)?;
            if total.is_some_and(|n| n < offset)
                || (expected_total.is_some() && expected_total != total)
            {
                return Err(DiscoveryError::IncompleteList);
            }
            if options.page_size.is_none() {
                if total.is_some_and(|n| n != offset) {
                    return Err(DiscoveryError::IncompleteList);
                }
                break;
            }
            // Pagination requires both documented counters; do not guess completeness.
            let total = total
                .filter(|_| count.is_some())
                .ok_or(DiscoveryError::IncompleteList)?;
            if rows > options.page_size.unwrap_or(0) {
                return Err(DiscoveryError::IncompleteList);
            }
            if offset == total {
                break;
            }
            if rows == 0 || report.pages >= options.max_pages {
                return Err(DiscoveryError::IncompleteList);
            }
            expected_total = Some(total);
        }
        if Instant::now() >= end {
            return Err(DiscoveryError::Deadline);
        }
        report.attempts = state.attempts;
        if report.endpoints.is_empty() {
            return Err(DiscoveryError::Empty);
        }
        Ok(report)
    }

    async fn request(
        &self,
        url: Url,
        state: &mut State,
        end: Instant,
    ) -> Result<reqwest::Response, DiscoveryError> {
        loop {
            state.reserve_attempt(&self.inner.options, end).await?;
            match self
                .inner
                .client
                .get(url.clone())
                .header("accept", "application/json")
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    if status == reqwest::StatusCode::OK {
                        return Ok(response);
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        if let Some(value) = response.headers().get("retry-after") {
                            let delay = retry_after(
                                value.to_str().map_err(|_| DiscoveryError::RetryAfter)?,
                                chrono::Utc::now(),
                            )?;
                            let next = Instant::now()
                                .checked_add(delay)
                                .ok_or(DiscoveryError::RetryAfter)?;
                            if next >= end {
                                return Err(DiscoveryError::RetryAfter);
                            }
                            state.next_attempt = state.next_attempt.max(next);
                        }
                        if state.attempts == self.inner.options.max_attempts {
                            return Err(DiscoveryError::Http(status.as_u16()));
                        }
                    } else {
                        return Err(DiscoveryError::Http(status.as_u16()));
                    }
                }
                Err(_) if state.attempts < self.inner.options.max_attempts => {}
                Err(_) => return Err(DiscoveryError::Transport),
            }
        }
    }
}
fn header_count(response: &reqwest::Response, name: &str) -> Result<Option<usize>, DiscoveryError> {
    response
        .headers()
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or(DiscoveryError::IncompleteList)
        })
        .transpose()
}
fn retry_after(
    value: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Duration, DiscoveryError> {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return value
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| DiscoveryError::RetryAfter);
    }
    let date =
        chrono::DateTime::parse_from_rfc2822(value).map_err(|_| DiscoveryError::RetryAfter)?;
    Ok((date.with_timezone(&chrono::Utc) - now)
        .to_std()
        .unwrap_or(Duration::ZERO))
}

#[derive(Deserialize)]
struct Row {
    ip: IpAddr,
    port: u16,
    protocols: Vec<String>,
    status: String,
    #[serde(alias = "country")]
    country_code: Option<String>,
    latency_ms: Option<f64>,
    uptime_pct: Option<f64>,
}
#[derive(Default)]
struct ParsedPage {
    duplicate_rows: usize,
    rows: usize,
    endpoints: Vec<ProxyEndpoint>,
    malformed_rows: usize,
    unsupported_rows: usize,
    filtered_rows: usize,
}
fn parse_page(bytes: &[u8], options: &DiscoveryOptions) -> Result<ParsedPage, DiscoveryError> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| DiscoveryError::InvalidJson)?;
    let mut page = ParsedPage {
        rows: rows.len(),
        ..Default::default()
    };
    let mut indexes = HashMap::new();
    for value in rows {
        let Ok(row) = serde_json::from_value::<Row>(value) else {
            page.malformed_rows += 1;
            continue;
        };
        if row.port == 0
            || row.ip.is_unspecified()
            || row.ip.is_multicast()
            || row
                .country_code
                .as_ref()
                .is_some_and(|s| s.len() != 2 || !s.bytes().all(|b| b.is_ascii_alphabetic()))
            || row.uptime_pct.is_some_and(|v| !(0.0..=100.0).contains(&v))
            || row.latency_ms.is_some_and(|v| v < 0.0)
        {
            page.malformed_rows += 1;
            continue;
        }
        let protocols: Vec<_> = [Protocol::Http, Protocol::Https]
            .into_iter()
            .filter(|p| row.protocols.iter().any(|v| v == p.label()))
            .collect();
        if protocols.is_empty() {
            page.unsupported_rows += 1;
            continue;
        }
        if !(row.status == "alive" || (options.include_recent && row.status == "recently_alive"))
            || !protocols.iter().any(|p| options.protocols.contains(p))
            || options.country.as_ref().is_some_and(|wanted| {
                !row.country_code
                    .as_ref()
                    .is_some_and(|c| c.eq_ignore_ascii_case(wanted))
            })
            || options
                .min_uptime_pct
                .is_some_and(|min| !row.uptime_pct.is_some_and(|v| v >= min))
            || options
                .max_latency_ms
                .is_some_and(|max| !row.latency_ms.is_some_and(|v| v <= f64::from(max)))
        {
            page.filtered_rows += 1;
            continue;
        }
        let address = SocketAddr::new(row.ip, row.port);
        if let Some(&index) = indexes.get(&address) {
            let existing: &mut ProxyEndpoint = &mut page.endpoints[index];
            for protocol in protocols {
                if !existing.protocols.contains(&protocol) {
                    existing.protocols.push(protocol);
                }
            }
            page.duplicate_rows += 1;
            continue;
        }
        indexes.insert(address, page.endpoints.len());
        // Input is byte bounded; additionally bound the output before returning to async code.
        if page.endpoints.len() == options.max_entries {
            return Err(DiscoveryError::TooManyEntries);
        }
        page.endpoints.push(ProxyEndpoint {
            address: SocketAddr::new(row.ip, row.port),
            protocols,
            country_code: row.country_code.map(|s| s.to_ascii_uppercase()),
            latency_ms: row.latency_ms,
            uptime_pct: row.uptime_pct,
        });
    }
    Ok(page)
}

#[cfg(test)]
mod tests;
