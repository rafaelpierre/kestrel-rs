//! Bounded asynchronous page retrieval and off-runtime HTML/plain-text extraction.

use std::sync::Arc;

use encoding_rs::Encoding;
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream::FuturesUnordered};
use once_cell::sync::Lazy;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use scraper::{ElementRef, Html, Selector};
use tokio::sync::Semaphore;

use crate::model::{FetchOptions, FetchOutcome, FetchReport, PageFetchDiagnostic};
use crate::search::KestrelError;

pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_000_000;
const SUPPORTED_CONTENT_TYPES: &[&str] = &["text/html", "application/xhtml+xml", "text/plain"];
// Whole attribute names only: arbitrary substrings (especially "ad") can
// identify article containers such as "download" and "thread".
const CLUTTER_MARKERS: &[&str] = &[
    "menu",
    "sidebar",
    "navbar",
    "topbar",
    "advertisement",
    "ad",
    "cookie",
    "modal",
    "popup",
    "banner",
    "nav",
    "breadcrumb",
    "ad-slot",
    "ad-container",
    "ad-banner",
    "cookie-banner",
    "cookie-consent",
    "sidebar-left",
    "sidebar-right",
];

static FIXED_CHROME: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("head, template, script, style, nav, header, footer, aside, form").unwrap()
});
static CONTAINERS: Lazy<Selector> =
    Lazy::new(|| Selector::parse("div, section").expect("valid selector"));
static MAIN: Lazy<Selector> = Lazy::new(|| Selector::parse("main").expect("valid selector"));
static ARTICLE: Lazy<Selector> = Lazy::new(|| Selector::parse("article").expect("valid selector"));
static ALL: Lazy<Selector> = Lazy::new(|| Selector::parse("*").expect("valid selector"));
/// Capacity retained by a reusable client and all of its clones.
pub(crate) struct ParserPool {
    semaphore: Arc<Semaphore>,
    #[cfg(test)]
    pub(crate) before_parse: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ParserPool {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(capacity)),
            #[cfg(test)]
            before_parse: None,
        }
    }
}

#[derive(Clone)]
struct Parsing {
    local: Arc<Semaphore>,
    shared: Option<Arc<ParserPool>>,
}

/// Fetch and parse URLs concurrently while preserving input order.
/// Each invocation owns independent capacity; reuse `KestrelClient` for a shared bound.
pub async fn fetch_all(
    urls: &[String],
    options: &FetchOptions,
) -> Result<Vec<Option<String>>, KestrelError> {
    let client = build_client()?;
    fetch_all_reusing_client(urls, options, &client).await
}

/// Fetch and parse URLs with phase-level diagnostics.
pub async fn fetch_all_detailed(
    urls: &[String],
    options: &FetchOptions,
) -> Result<FetchReport, KestrelError> {
    let client = build_client()?;
    fetch_all_reusing_client_with_diagnostics(urls, options, &client, None).await
}

pub(crate) fn build_client() -> Result<reqwest::Client, KestrelError> {
    build_client_with_transport(&crate::TransportOptions::default())
}

pub(crate) fn build_client_with_transport(
    transport: &crate::TransportOptions,
) -> Result<reqwest::Client, KestrelError> {
    transport.validate()?;
    let profile = crate::http_client::BrowserProfile::random();
    crate::benchmarking::capture_headers("fetch", &profile.headers());
    Ok(crate::http_client::standard_builder(profile, transport).build()?)
}

pub(crate) async fn fetch_all_reusing_client(
    urls: &[String],
    options: &FetchOptions,
    client: &reqwest::Client,
) -> Result<Vec<Option<String>>, KestrelError> {
    Ok(
        fetch_all_reusing_client_with_diagnostics(urls, options, client, None)
            .await?
            .contents,
    )
}

pub(crate) async fn fetch_all_reusing_client_with_diagnostics(
    urls: &[String],
    options: &FetchOptions,
    client: &reqwest::Client,
    budget: Option<Duration>,
) -> Result<FetchReport, KestrelError> {
    fetch_all_with_parser_pool(urls, options, client, budget, None).await
}

pub(crate) async fn fetch_all_with_parser_pool(
    urls: &[String],
    options: &FetchOptions,
    client: &reqwest::Client,
    budget: Option<Duration>,
    shared: Option<&Arc<ParserPool>>,
) -> Result<FetchReport, KestrelError> {
    crate::telemetry::scope_result("kestrel.fetch", async {
        crate::telemetry::payload("fetch.input", urls);
        crate::telemetry::attribute("kestrel.timeout_seconds", options.timeout.as_secs_f64());
        crate::telemetry::attribute("kestrel.fetch_concurrency", options.max_concurrency as i64);
        crate::telemetry::attribute(
            "kestrel.parse_concurrency",
            options.parse_concurrency as i64,
        );
        if let Some(budget) = budget {
            crate::telemetry::attribute("kestrel.fetch_budget_seconds", budget.as_secs_f64());
        }
        crate::telemetry::attribute("kestrel.content_limit", options.content_limit as i64);
        crate::telemetry::attribute(
            "kestrel.max_response_bytes",
            options.max_response_bytes as i64,
        );
        validate_options(options)?;
        let deadline = budget
            .map(|value| crate::numeric::deadline("fetch budget", value))
            .transpose()?;
        let network = Arc::new(Semaphore::new(options.max_concurrency));
        let parsing = Parsing {
            local: Arc::new(Semaphore::new(options.parse_concurrency)),
            shared: shared.cloned(),
        };
        let mut jobs: FuturesUnordered<_> = urls
            .iter()
            .enumerate()
            .map(|(index, url)| {
                let network = Arc::clone(&network);
                let parsing = parsing.clone();
                async move {
                    (
                        index,
                        fetch_one_detailed(url, client, network, parsing, options).await,
                    )
                }
            })
            .collect();
        let mut results = vec![None; urls.len()];
        let mut diagnostics = vec![None; urls.len()];
        let mut budget_exhausted = false;
        let mut cancelled = 0;
        if let Some(deadline) = deadline {
            let deadline = tokio::time::sleep_until(deadline);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    item = jobs.next() => {
                        let Some((index, item)) = item else { break };
                        results[index] = item.content;
                        diagnostics[index] = Some(item.diagnostic);
                    }
                    () = &mut deadline => {
                        budget_exhausted = !jobs.is_empty();
                        cancelled = jobs.len();
                        break;
                    },
                }
            }
        } else {
            while let Some((index, item)) = jobs.next().await {
                results[index] = item.content;
                diagnostics[index] = Some(item.diagnostic);
            }
        }
        crate::telemetry::attribute("kestrel.budget_exhausted", budget_exhausted);
        crate::telemetry::attribute("kestrel.cancelled_pages", cancelled as i64);
        Ok(FetchReport {
            contents: results,
            pages: diagnostics.into_iter().flatten().collect(),
            budget_exhausted,
            cancelled,
            cache_hits: 0,
            cache_misses: urls.len(),
        })
    })
    .await
}

// Append only the prefix that fits. Return false as soon as the cap is reached,
// even at an exact boundary: polling again could wait indefinitely for EOF.
fn append_body_chunk(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> bool {
    let chunk = &chunk[..chunk.len().min(limit.saturating_sub(body.len()))];
    if body.capacity() - body.len() < chunk.len() {
        let capacity = (body.len() + chunk.len())
            .max(body.capacity().saturating_mul(2))
            .min(limit);
        body.reserve_exact(capacity - body.len());
    }
    body.extend_from_slice(chunk);
    body.len() < limit
}

pub(crate) fn validate_options(options: &FetchOptions) -> Result<(), KestrelError> {
    for (name, value) in [
        ("max_concurrency", options.max_concurrency),
        ("parse_concurrency", options.parse_concurrency),
        ("max_response_bytes", options.max_response_bytes),
        ("content_limit", options.content_limit),
    ] {
        if value < 1 {
            return Err(KestrelError::InvalidRequest(format!(
                "{name} must be at least 1"
            )));
        }
    }
    crate::numeric::concurrency("max_concurrency", options.max_concurrency)?;
    crate::numeric::concurrency("parse_concurrency", options.parse_concurrency)?;
    crate::numeric::duration("timeout", options.timeout)?;
    Ok(())
}

enum ContentKind {
    Html,
    PlainText,
}

struct FetchItem {
    content: Option<String>,
    diagnostic: PageFetchDiagnostic,
}

async fn fetch_one_detailed(
    url: &str,
    client: &reqwest::Client,
    network: Arc<Semaphore>,
    parsing: Parsing,
    options: &FetchOptions,
) -> FetchItem {
    crate::telemetry::scope("kestrel.page", async {
        crate::telemetry::payload("page.input", &url);
        let started = Instant::now();
        let mut http_version = None;
        let mut item = match fetch_one_inner(
            url,
            client,
            network,
            parsing,
            options,
            started,
            &mut http_version,
        )
        .await
        {
            Ok(item) => item,
            Err(error) => {
                crate::log_event!(
                    "fetch_failed",
                    "url" => url,
                    "error_type" => "request",
                    "error" => error.to_string(),
                );
                FetchItem {
                    content: None,
                    diagnostic: PageFetchDiagnostic {
                        url: url.to_owned(),
                        outcome: FetchOutcome::RequestFailed,
                        http_version: None,
                        queue_ms: 0,
                        request_ms: 0,
                        download_ms: 0,
                        parse_queue_ms: 0,
                        parse_ms: 0,
                        total_ms: elapsed_millis(started),
                        response_bytes: 0,
                    },
                }
            }
        };
        item.diagnostic.http_version = http_version;
        #[cfg(test)]
        if let Some(content) = &item.content {
            crate::recovery_audit::observe("page-extracted", content);
        }
        crate::telemetry::payload("page.output", &item.content);
        crate::telemetry::payload("page.diagnostic", &item.diagnostic);
        crate::telemetry::attribute("kestrel.outcome", format!("{:?}", item.diagnostic.outcome));
        crate::telemetry::attribute(
            "kestrel.response_bytes",
            item.diagnostic.response_bytes as i64,
        );
        if item.content.is_none() {
            crate::telemetry::error("page_fetch_failed");
        }
        item
    })
    .await
}

async fn fetch_one_inner(
    url: &str,
    client: &reqwest::Client,
    network: Arc<Semaphore>,
    parsing: Parsing,
    options: &FetchOptions,
    started: Instant,
    http_version: &mut Option<String>,
) -> Result<FetchItem, KestrelError> {
    let queue_started = Instant::now();
    // Keep the download slot until a parser takes ownership of this body.
    // This bounds downloading/waiting bodies to max_concurrency per batch.
    let network_permit = crate::telemetry::scope("kestrel.queue", network.acquire())
        .await
        .expect("semaphore remains open");
    let mut attempt = crate::telemetry::Span::new("kestrel.http_attempt");
    let (body, encoding, content_kind, queue_ms, request_ms, download_ms, response_bytes) = {
        let queue_ms = elapsed_millis(queue_started);
        let request_started = Instant::now();
        use opentelemetry::trace::FutureExt;
        let response = crate::telemetry::scope(
            "kestrel.send",
            client.get(url).timeout(options.timeout).send(),
        )
        .with_context(attempt.context())
        .await?;
        attempt.attribute(
            "http.response.status_code",
            i64::from(response.status().as_u16()),
        );
        *http_version = Some(format!("{:?}", response.version()));
        let response = response.error_for_status()?;
        let request_ms = elapsed_millis(request_started);
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.is_empty()
            && !SUPPORTED_CONTENT_TYPES
                .iter()
                .any(|supported| content_type.starts_with(supported))
        {
            crate::log_event!(
                "fetch_skipped",
                "url" => url,
                "reason" => "unsupported_content_type",
                "content_type" => content_type,
            );
            return Ok(FetchItem {
                content: None,
                diagnostic: page_diagnostic(
                    url,
                    FetchOutcome::UnsupportedContentType,
                    started,
                    queue_ms,
                    request_ms,
                    0,
                    0,
                    0,
                    0,
                ),
            });
        }
        let declared_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .map(|length| {
                length
                    .to_str()
                    .map_err(|error| KestrelError::Search(error.to_string()))?
                    .parse::<usize>()
                    .map_err(|error| KestrelError::Search(error.to_string()))
            })
            .transpose()?;
        let parsed_type = content_type.parse::<mime::Mime>().ok();
        let encoding = response_encoding(parsed_type.as_ref());
        let content_kind = if parsed_type
            .as_ref()
            .is_some_and(|value| value.essence_str() == "text/plain")
        {
            ContentKind::PlainText
        } else {
            ContentKind::Html
        };
        let mut body_span = crate::telemetry::Span::with_parent("kestrel.body", &attempt.context());
        let download_started = Instant::now();
        let mut stream = response.bytes_stream();
        let mut body = Vec::with_capacity(
            declared_length
                .unwrap_or_default()
                .min(options.max_response_bytes),
        );
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if !append_body_chunk(&mut body, &chunk, options.max_response_bytes) {
                crate::log_event!(
                    "fetch_capped",
                    "url" => url,
                    "response_bytes" => body.len(),
                    "max_response_bytes" => options.max_response_bytes,
                );
                break;
            }
        }
        // Drop the unread response before parser admission. On HTTP/2 this
        // cancels this stream while allowing other pooled streams to continue.
        drop(stream);
        let download_ms = elapsed_millis(download_started);
        let response_bytes = body.len();
        body_span.finish();
        attempt.finish();
        (
            body,
            encoding,
            content_kind,
            queue_ms,
            request_ms,
            download_ms,
            response_bytes,
        )
    };

    drop(attempt);
    let limit = options.content_limit;
    let parse_queue_started = Instant::now();
    let parse_permit = parsing
        .local
        .acquire_owned()
        .await
        .expect("semaphore remains open");
    // Always acquire local then shared capacity; no path acquires in reverse.
    // Keep download backpressure until both slots belong to the blocking job.
    let shared_permit = if let Some(pool) = &parsing.shared {
        Some(
            pool.semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore remains open"),
        )
    } else {
        None
    };
    drop(network_permit);
    let parse_queue_ms = elapsed_millis(parse_queue_started);
    let parse_started = Instant::now();
    let context = crate::telemetry::parent_context();
    let content = tokio::task::spawn_blocking(move || {
        let _context = context.attach();
        crate::telemetry::scope_sync("kestrel.extraction", || {
            // Blocking work survives cancellation of its async caller. Keep its
            // slot until the body and DOM are released, including while queued.
            let _permit = parse_permit;
            let _shared_permit = shared_permit;
            let body = body;
            #[cfg(test)]
            if let Some(hook) = parsing
                .shared
                .as_ref()
                .and_then(|pool| pool.before_parse.as_ref())
            {
                hook();
            }
            let (text, _, _) = encoding.decode(&body);
            match content_kind {
                ContentKind::Html => parse_content(&text, limit),
                ContentKind::PlainText => {
                    // Preserve literal markup, whitespace, and repeated lines. Test
                    // the retained prefix so a whitespace-only cutoff is NoContent.
                    let retained: String = text.chars().take(limit).collect();
                    (!retained.trim().is_empty()).then_some(retained)
                }
            }
        })
    })
    .await
    .map_err(|error| KestrelError::Search(format!("HTML parser task failed: {error}")))?;
    let parse_ms = elapsed_millis(parse_started);
    crate::telemetry::scope_sync("kestrel.content_quality", || {
        crate::telemetry::payload(
            "quality",
            &crate::assess_content_quality(content.as_deref()),
        );
    });
    let outcome = if content.is_some() {
        FetchOutcome::Success
    } else {
        FetchOutcome::NoContent
    };
    Ok(FetchItem {
        content,
        diagnostic: page_diagnostic(
            url,
            outcome,
            started,
            queue_ms,
            request_ms,
            download_ms,
            parse_queue_ms,
            parse_ms,
            response_bytes,
        ),
    })
}

#[allow(clippy::too_many_arguments)]
fn page_diagnostic(
    url: &str,
    outcome: FetchOutcome,
    started: Instant,
    queue_ms: u64,
    request_ms: u64,
    download_ms: u64,
    parse_queue_ms: u64,
    parse_ms: u64,
    response_bytes: usize,
) -> PageFetchDiagnostic {
    PageFetchDiagnostic {
        url: url.to_owned(),
        outcome,
        http_version: None,
        queue_ms,
        request_ms,
        download_ms,
        parse_queue_ms,
        parse_ms,
        total_ms: elapsed_millis(started),
        response_bytes,
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn response_encoding(content_type: Option<&mime::Mime>) -> &'static Encoding {
    content_type
        .and_then(|value| {
            value
                .get_param(mime::CHARSET)
                .map(|charset| charset.as_str().to_owned())
        })
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8)
}

pub(crate) fn parse_content(html: &str, content_limit: usize) -> Option<String> {
    let mut document = Html::parse_document(html);
    remove_page_chrome(&mut document);
    let root = main_content(&document);
    let content = extract_text(root, content_limit);
    // Only reconsider an entirely recognized shell. Inspect at most the first
    // explicit article, and assess the bounded retained text without truncating
    // an over-limit assessment. Ordinary root selection remains unchanged.
    if crate::assess_content_quality(content.as_deref()).state
        == crate::ContentQualityState::BoilerplateOnly
        && let Some(article) = document.select(&ARTICLE).next()
        && article.id() != root.id()
    {
        let alternative = extract_text(article, content_limit);
        let quality = crate::assess_content_quality(alternative.as_deref());
        if alternative.is_some()
            && (quality.state == crate::ContentQualityState::Unflagged
                || quality.reasons == [crate::ContentQualityReason::MixedContent])
        {
            return alternative;
        }
    }
    content
}

fn remove_page_chrome(document: &mut Html) {
    let mut ids: Vec<_> = document
        .select(&FIXED_CHROME)
        .map(|element| element.id())
        .collect();
    ids.extend(document.select(&CONTAINERS).filter_map(|element| {
        let value = element.value();
        let class_is_chrome = value.classes().any(is_chrome_marker);
        let id_is_chrome = value.attr("id").is_some_and(|id| {
            is_chrome_marker(id)
                || id.rsplit_once(['-', '_']).is_some_and(|(marker, suffix)| {
                    // Repeated slots may have numeric IDs, e.g. ad-123. Do not
                    // generalize this to arbitrary words such as ad-supported.
                    is_chrome_marker(marker)
                        && !suffix.is_empty()
                        && suffix.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
        (class_is_chrome || id_is_chrome).then(|| element.id())
    }));
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
    }
}

fn is_chrome_marker(name: &str) -> bool {
    CLUTTER_MARKERS
        .iter()
        .any(|marker| name.eq_ignore_ascii_case(marker))
}

fn main_content(document: &Html) -> ElementRef<'_> {
    for query in [&*MAIN, &*ARTICLE] {
        if let Some(element) = document.select(query).next() {
            return element;
        }
    }
    if let Some(element) = document.select(&ALL).find(|element| {
        let classes = element
            .value()
            .attr("class")
            .unwrap_or_default()
            .to_ascii_lowercase();
        ["content", "main", "post", "body"]
            .iter()
            .any(|pattern| classes.contains(pattern))
    }) {
        return element;
    }
    document.root_element()
}

/// Walk the selected DOM once in source order, without recursive calls or
/// duplicated ancestor text. The output allocation is bounded by the character
/// limit; the already bounded HTML parser owns the DOM allocation.
fn extract_text(root: ElementRef<'_>, content_limit: usize) -> Option<String> {
    let mut output = TextOutput::new(content_limit);
    let mut node = *root;
    let mut entering = true;
    let mut pre_depth = 0usize;
    loop {
        if let Some(element) = ElementRef::wrap(node) {
            let name = element.value().name();
            if name == "pre" {
                if entering {
                    output.boundary('\n');
                    pre_depth += 1;
                } else {
                    pre_depth -= 1;
                    output.boundary('\n');
                }
            } else if pre_depth == 0 {
                if is_text_block(name) || name == "br" {
                    output.boundary('\n');
                } else if entering
                    && matches!(name, "td" | "th")
                    && element
                        .prev_siblings()
                        .filter_map(ElementRef::wrap)
                        .any(|sibling| matches!(sibling.value().name(), "td" | "th"))
                {
                    if output.pending == Some(' ') {
                        output.pending = None;
                    }
                    output.literal("\t");
                }
            } else if entering && name == "br" {
                output.literal("\n");
            }
        } else if entering && let Some(text) = node.value().as_text() {
            if pre_depth > 0 {
                output.literal(text);
            } else {
                output.prose(text);
            }
        }
        if output.remaining == 0 {
            break;
        }
        if entering && let Some(child) = node.first_child() {
            node = child;
            continue;
        }
        if entering {
            entering = false;
            continue;
        }
        if node.id() == root.id() {
            break;
        }
        if let Some(sibling) = node.next_sibling() {
            node = sibling;
            entering = true;
        } else if let Some(parent) = node.parent() {
            node = parent;
        } else {
            break;
        }
    }
    output
        .text
        .chars()
        .any(|c| !c.is_whitespace())
        .then_some(output.text)
}

fn is_text_block(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "blockquote"
            | "caption"
            | "dd"
            | "details"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "li"
            | "main"
            | "ol"
            | "p"
            | "section"
            | "summary"
            | "table"
            | "tr"
            | "ul"
    )
}

struct TextOutput {
    text: String,
    remaining: usize,
    pending: Option<char>,
}

impl TextOutput {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            remaining: limit,
            pending: None,
        }
    }

    // Delay generated separators so empty wrappers and trailing markup do not
    // add blank lines or consume the content budget.
    fn boundary(&mut self, separator: char) {
        if self.pending != Some('\n') {
            self.pending = Some(separator);
        }
    }

    fn push(&mut self, c: char) {
        if self.remaining > 0 {
            self.text.push(c);
            self.remaining -= 1;
        }
    }

    fn flush(&mut self) {
        if let Some(separator) = self.pending.take()
            && !self.text.is_empty()
            && !self.text.ends_with('\n')
            && (separator == '\n' || !self.text.ends_with([' ', '\t']))
        {
            self.push(separator);
        }
    }

    fn literal(&mut self, text: &str) {
        if !text.is_empty() {
            self.flush();
            for c in text.chars().take(self.remaining) {
                self.text.push(c);
                self.remaining -= 1;
            }
        }
    }

    fn prose(&mut self, text: &str) {
        for c in text.chars() {
            if self.remaining == 0 {
                break;
            }
            // HTML whitespace collapses across text nodes, never independently
            // at inline element boundaries. Entities are already decoded by DOM
            // parsing; decoding again would corrupt literal &amp; examples.
            if c.is_whitespace() {
                self.boundary(' ');
            } else {
                self.flush();
                self.push(c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><body><nav>Ignore navigation</nav><aside>Ignore sidebar</aside><main><h1>Kestrel heading</h1><h2>Useful section</h2><p>This is meaningful page content that should be preserved in the extraction.</p><p>Source: ignored metadata</p></main></body></html>"#;

    #[test]
    fn retains_crossing_chunk_prefix_without_exceeding_cap() {
        let _telemetry = crate::telemetry::test_export_guard();
        let mut body = vec![b'x'; 15];
        assert!(!append_body_chunk(&mut body, &[b'y'; 100], 16));
        assert_eq!(body, [vec![b'x'; 15], vec![b'y']].concat());
        assert_eq!(body.capacity(), 16);
        assert!(!append_body_chunk(&mut body, b"z", 16));
        assert_eq!(body.len(), 16);
        let mut body = Vec::new();
        assert!(append_body_chunk(&mut body, b"x", 2));
        assert!(!append_body_chunk(&mut body, b"y", 2));
        assert_eq!(body, b"xy");
    }

    #[test]
    fn parser_backpressure_bounds_large_batches_and_survives_cancellation() {
        let _telemetry = crate::telemetry::test_export_guard();
        // Occupy the only blocking worker so parsers deterministically retain
        // their bodies without finishing. No timing assumptions about HTML CPU.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

            let server = MockServer::start().await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let options = FetchOptions {
                max_concurrency: 5,
                parse_concurrency: 2,
                max_response_bytes: 64 * 1024,
                ..FetchOptions::default()
            };
            let page = format!("<main><p>{}</p></main>", "retained page text ".repeat(3400));
            assert!(page.len() <= options.max_response_bytes);
            assert!(page.len() > options.max_response_bytes * 9 / 10);
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/html")
                        .set_body_bytes(page.clone()),
                )
                .mount(&server)
                .await;

            for (batch_size, cancel) in [(200, false), (400, true)] {
                let previous_requests = server.received_requests().await.unwrap().len();
                let (release, gate) = std::sync::mpsc::channel::<()>();
                let (entered, ready) = tokio::sync::oneshot::channel();
                let blocker = tokio::task::spawn_blocking(move || {
                    entered.send(()).unwrap();
                    // Sender drops on test failure, preventing shutdown hangs.
                    let _ = gate.recv();
                });
                ready.await.unwrap();
                let network = Arc::new(Semaphore::new(options.max_concurrency));
                let parsing = Arc::new(Semaphore::new(options.parse_concurrency));
                let mut jobs = Vec::new();
                for index in 0..batch_size {
                    let url = format!("{}/{index}", server.uri());
                    let client = client.clone();
                    let network = network.clone();
                    let parsing = parsing.clone();
                    let options = options.clone();
                    jobs.push(tokio::spawn(async move {
                        fetch_one_detailed(
                            &url,
                            &client,
                            network,
                            Parsing {
                                local: parsing,
                                shared: None,
                            },
                            &options,
                        )
                        .await
                    }));
                }
                let bound = options.max_concurrency + options.parse_concurrency;
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let requests =
                            server.received_requests().await.unwrap().len() - previous_requests;
                        if requests >= bound && parsing.available_permits() == 0 {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                // Let the fast server drain any erroneously admitted downloads.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let requests = server.received_requests().await.unwrap().len() - previous_requests;
                assert_eq!(requests, bound, "batch size {batch_size}");
                assert!(requests * page.len() <= bound * options.max_response_bytes);
                assert_eq!(network.available_permits(), 0);

                if cancel {
                    for job in &jobs {
                        job.abort();
                    }
                    for job in jobs {
                        assert!(matches!(job.await, Err(error) if error.is_cancelled()));
                    }
                    assert_eq!(network.available_permits(), options.max_concurrency);
                    // The queued blocking parsers still own their bodies/slots.
                    assert_eq!(parsing.available_permits(), 0);
                    drop(release);
                    blocker.await.unwrap();
                } else {
                    drop(release);
                    blocker.await.unwrap();
                    for (index, job) in jobs.into_iter().enumerate() {
                        let item = job.await.unwrap();
                        assert_eq!(item.diagnostic.url, format!("{}/{index}", server.uri()));
                        assert_eq!(item.diagnostic.outcome, FetchOutcome::Success);
                        assert!(item.content.unwrap().contains("retained page text"));
                    }
                }
                tokio::time::timeout(Duration::from_secs(5), async {
                    while parsing.available_permits() != options.parse_concurrency {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(network.available_permits(), options.max_concurrency);
            }
        });
    }

    #[test]
    fn client_parser_capacity_survives_started_work_and_repeated_calls() {
        let _telemetry = crate::telemetry::test_export_guard();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use std::sync::{
                Mutex,
                atomic::{AtomicUsize, Ordering},
            };
            use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string(PAGE))
                .mount(&server)
                .await;
            let mut client = crate::KestrelClient::with_parser_capacity(2).unwrap();
            client.fetch = reqwest::Client::builder().no_proxy().build().unwrap();
            let started = Arc::new(AtomicUsize::new(0));
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let gate = Arc::new(Mutex::new(gate));
            let mut pool = ParserPool::new(2);
            pool.before_parse = Some(Arc::new({
                let started = started.clone();
                move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    // Disconnect on panic also releases every blocked parser.
                    let _ = gate.lock().unwrap().recv();
                }
            }));
            client.parsing = Arc::new(pool);
            let options = FetchOptions {
                parse_concurrency: 1,
                ..FetchOptions::default()
            };
            let urls = vec![server.uri(); 4];
            let first = tokio::spawn({
                let client = client.clone();
                let urls = urls.clone();
                let options = options.clone();
                async move {
                    client
                        .fetch_all_detailed(&urls, &options, Some(Duration::from_millis(500)))
                        .await
                        .unwrap()
                }
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while started.load(Ordering::SeqCst) != 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert_eq!(
                started.load(Ordering::SeqCst),
                1,
                "per-call limit still applies"
            );
            assert_eq!(client.parsing.semaphore.available_permits(), 1);
            let wider = FetchOptions {
                parse_concurrency: 8,
                ..options.clone()
            };
            let second = tokio::spawn({
                let client = client.clone();
                let urls = urls.clone();
                let options = wider.clone();
                async move { client.fetch_all(&urls, &options).await }
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while started.load(Ordering::SeqCst) != 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            second.abort();
            assert!(second.await.unwrap_err().is_cancelled());
            // Measure API completion while the actual parsers remain blocked,
            // independently from runtime shutdown (which happens after release).
            let report = tokio::time::timeout(Duration::from_secs(2), first)
                .await
                .unwrap()
                .unwrap();
            assert!(report.budget_exhausted);
            assert_eq!(report.cancelled, urls.len());
            assert_eq!(client.parsing.semaphore.available_permits(), 0);
            let directory = tempfile::tempdir().unwrap();
            let cache = crate::PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
            for _ in 0..3 {
                let budget = Duration::from_millis(30);
                assert!(
                    client
                        .clone()
                        .fetch_all_with_budget(&urls, &wider, budget)
                        .await
                        .unwrap()
                        .iter()
                        .all(Option::is_none)
                );
                let report = client
                    .clone()
                    .fetch_all_cached_detailed(&urls, &wider, &cache, Some(budget))
                    .await
                    .unwrap();
                assert!(report.budget_exhausted);
                assert_eq!(started.load(Ordering::SeqCst), 2);
                assert_eq!(client.parsing.semaphore.available_permits(), 0);
            }
            let pending = tokio::spawn({
                let client = client.clone();
                let urls = urls.clone();
                async move { client.fetch_all(&urls, &wider).await.unwrap() }
            });
            drop(release);
            let contents = tokio::time::timeout(Duration::from_secs(5), pending)
                .await
                .unwrap()
                .unwrap();
            assert!(contents.iter().all(|text| {
                text.as_ref()
                    .is_some_and(|text| text.contains("Kestrel heading"))
            }));
            assert_eq!(client.parsing.semaphore.available_permits(), 2);
        });
        runtime.shutdown_timeout(Duration::from_secs(5));
    }

    #[test]
    fn client_parser_capacity_bounds_queued_jobs_across_clones() {
        let _telemetry = crate::telemetry::test_export_guard();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string(PAGE))
                .mount(&server)
                .await;
            let mut client = crate::KestrelClient::with_parser_capacity(2).unwrap();
            client.fetch = reqwest::Client::builder().no_proxy().build().unwrap();
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let (entered, ready) = tokio::sync::oneshot::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                let _ = gate.recv();
            });
            ready.await.unwrap();
            let urls = vec![server.uri(); 20];
            let options = FetchOptions {
                max_concurrency: 2,
                parse_concurrency: 1,
                ..FetchOptions::default()
            };
            for _ in 0..4 {
                let report = client
                    .clone()
                    .fetch_all_detailed(&urls, &options, Some(Duration::from_millis(100)))
                    .await
                    .unwrap();
                assert!(report.budget_exhausted);
            }
            assert_eq!(client.parsing.semaphore.available_permits(), 0);
            // Two admitted parsers plus two downloading/waiting bodies per call.
            assert_eq!(server.received_requests().await.unwrap().len(), 2 + 4 * 2);
            drop(release);
            blocker.await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while client.parsing.semaphore.available_permits() != 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(client.fetch_all(&urls[..1], &options).await.unwrap()[0].is_some());
        });
        runtime.shutdown_timeout(Duration::from_secs(5));
    }

    #[test]
    fn extracts_main_text_and_preserves_metadata() {
        let _telemetry = crate::telemetry::test_export_guard();
        let content = parse_content(PAGE, 500).unwrap();
        assert!(content.contains("Kestrel heading"));
        assert!(content.contains("Useful section"));
        assert!(content.contains("meaningful page content"));
        assert!(!content.contains("Ignore navigation"));
        assert!(content.contains("Source: ignored metadata"));
    }

    #[test]
    fn rejects_documents_without_meaningful_text() {
        let _telemetry = crate::telemetry::test_export_guard();
        assert_eq!(
            parse_content(
                "<html><head><title>Only metadata</title></head><body><p> </p></body></html>",
                100
            ),
            None
        );
    }

    #[test]
    fn legitimate_container_names_retain_nested_main_content() {
        let _telemetry = crate::telemetry::test_export_guard();
        let text = "This useful download instruction must remain visible in the extracted content.";
        for name in [
            "download",
            "reader",
            "shadow",
            "thread",
            "menuitem-docs",
            "navigation-guide",
            "ad-supported",
            "cookie-api",
            "modal-theory",
            "ad-",
            "ad-12x",
            "ad_guide",
            "ad-１２",
        ] {
            for attribute in ["class", "id"] {
                for html in [
                    format!(r#"<main><div {attribute}="{name}"><p>{text}</p></div></main>"#),
                    format!(
                        r#"<section {attribute}="{name}"><div><main><p>{text}</p></main></div></section>"#
                    ),
                ] {
                    assert_eq!(parse_content(&html, 500).as_deref(), Some(text), "{html}");
                }
            }
        }
    }

    #[test]
    fn explicit_chrome_markers_remove_meaningful_nested_text() {
        let _telemetry = crate::telemetry::test_export_guard();
        let text = "The useful documentation paragraph remains available after chrome removal.";
        for attributes in [
            r#"class="ad""#,
            r#"class="layout AD highlighted""#,
            r#"class="sidebar""#,
            r#"class="nav""#,
            r#"id="advertisement""#,
            r#"id="ad-slot""#,
            r#"id="ad-123""#,
            r#"id="sidebar_2""#,
            r#"class="cookie-banner""#,
        ] {
            let html = format!(
                r#"<main><div {attributes}><section><p>Unwanted advertising and navigation content must be removed.</p></section></div><p>{text}</p></main>"#
            );
            assert_eq!(parse_content(&html, 500).as_deref(), Some(text), "{html}");
        }
    }

    #[test]
    fn documentation_fixtures_retain_body_and_exclude_chrome() {
        let _telemetry = crate::telemetry::test_export_guard();
        for (html, expected) in [
            (
                include_str!("../tests/fixtures/extraction/book.html"),
                include_str!("../tests/fixtures/extraction/book.txt"),
            ),
            (
                include_str!("../tests/fixtures/extraction/api.html"),
                include_str!("../tests/fixtures/extraction/api.txt"),
            ),
            (
                include_str!("../tests/fixtures/extraction/download.html"),
                include_str!("../tests/fixtures/extraction/download.txt"),
            ),
        ] {
            assert_eq!(
                parse_content(html, 20_000).as_deref(),
                Some(expected.trim())
            );
        }
    }

    #[test]
    fn nested_clutter_is_removed() {
        let _telemetry = crate::telemetry::test_export_guard();
        let html = r#"<main><div class="sidebar"><section>Ignored nested clutter</section></div><p>This meaningful content remains available after nested clutter is removed.</p></main>"#;
        assert_eq!(
            parse_content(html, 500).as_deref(),
            Some("This meaningful content remains available after nested clutter is removed.")
        );
    }
}

#[cfg(test)]
mod cutoff_tests;

#[cfg(test)]
mod quality_tests;

#[cfg(test)]
mod ordered_tests;
