//! Bounded asynchronous page retrieval and off-runtime HTML extraction.

use std::sync::Arc;

use encoding_rs::Encoding;
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream::FuturesUnordered};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use scraper::{ElementRef, Html, Selector};
use tokio::sync::Semaphore;

use crate::model::{FetchOptions, FetchOutcome, FetchReport, PageFetchDiagnostic};
use crate::search::KestrelError;

pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_000_000;
const SUPPORTED_CONTENT_TYPES: &[&str] = &["text/html", "application/xhtml+xml", "text/plain"];
const CLUTTER_PATTERNS: &[&str] = &[
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
];

static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("valid whitespace regex"));
static NOISE: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"^Source:",
        r"^Status:",
        r"^Use Case:",
        r"^Description:",
        r"^Stars:",
        r"^Verified:",
        r"^Purpose:",
        r"^Portability:",
        r"^Token Cost:",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).expect("valid noise regex"))
    .collect()
});
static FIXED_CHROME: Lazy<Selector> =
    Lazy::new(|| Selector::parse("script, style, nav, header, footer, aside, form").unwrap());
static CONTAINERS: Lazy<Selector> =
    Lazy::new(|| Selector::parse("div, section").expect("valid selector"));
static MAIN: Lazy<Selector> = Lazy::new(|| Selector::parse("main").expect("valid selector"));
static ARTICLE: Lazy<Selector> = Lazy::new(|| Selector::parse("article").expect("valid selector"));
static ALL: Lazy<Selector> = Lazy::new(|| Selector::parse("*").expect("valid selector"));
static WEIGHTED_TEXT: Lazy<Vec<(Selector, usize, usize)>> = Lazy::new(|| {
    [("h1", 3, 5), ("h2", 2, 5), ("p", 1, 20), ("li", 1, 20)]
        .into_iter()
        .map(|(tag, weight, minimum)| {
            (
                Selector::parse(tag).expect("valid selector"),
                weight,
                minimum,
            )
        })
        .collect()
});

/// Fetch and parse URLs concurrently while preserving input order.
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

pub(crate) async fn fetch_all_reusing_client_with_budget(
    urls: &[String],
    options: &FetchOptions,
    client: &reqwest::Client,
    budget: Option<Duration>,
) -> Result<Vec<Option<String>>, KestrelError> {
    Ok(
        fetch_all_reusing_client_with_diagnostics(urls, options, client, budget)
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
    validate_options(options)?;
    let network = Arc::new(Semaphore::new(options.max_concurrency));
    let parsing = Arc::new(Semaphore::new(options.parse_concurrency));
    let mut jobs: FuturesUnordered<_> = urls
        .iter()
        .enumerate()
        .map(|(index, url)| {
            let network = Arc::clone(&network);
            let parsing = Arc::clone(&parsing);
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
    if let Some(budget) = budget {
        let deadline = tokio::time::sleep(budget);
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
    Ok(FetchReport {
        contents: results,
        pages: diagnostics.into_iter().flatten().collect(),
        budget_exhausted,
        cancelled,
        cache_hits: 0,
        cache_misses: urls.len(),
    })
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

fn validate_options(options: &FetchOptions) -> Result<(), KestrelError> {
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
    if options.timeout.is_zero() {
        return Err(KestrelError::InvalidRequest(
            "timeout must be greater than zero".into(),
        ));
    }
    Ok(())
}

struct FetchItem {
    content: Option<String>,
    diagnostic: PageFetchDiagnostic,
}

async fn fetch_one_detailed(
    url: &str,
    client: &reqwest::Client,
    network: Arc<Semaphore>,
    parsing: Arc<Semaphore>,
    options: &FetchOptions,
) -> FetchItem {
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
    item
}

async fn fetch_one_inner(
    url: &str,
    client: &reqwest::Client,
    network: Arc<Semaphore>,
    parsing: Arc<Semaphore>,
    options: &FetchOptions,
    started: Instant,
    http_version: &mut Option<String>,
) -> Result<FetchItem, KestrelError> {
    let queue_started = Instant::now();
    // Keep the download slot until a parser takes ownership of this body.
    // This bounds downloading/waiting bodies to max_concurrency per batch.
    let network_permit = network.acquire().await.expect("semaphore remains open");
    let (body, encoding, queue_ms, request_ms, download_ms, response_bytes) = {
        let queue_ms = elapsed_millis(queue_started);
        let request_started = Instant::now();
        let response = client.get(url).timeout(options.timeout).send().await?;
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
        let encoding = response_encoding(&content_type);
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
        (
            body,
            encoding,
            queue_ms,
            request_ms,
            download_ms,
            response_bytes,
        )
    };

    let limit = options.content_limit;
    let parse_queue_started = Instant::now();
    let parse_permit = parsing
        .acquire_owned()
        .await
        .expect("semaphore remains open");
    drop(network_permit);
    let parse_queue_ms = elapsed_millis(parse_queue_started);
    let parse_started = Instant::now();
    let content = tokio::task::spawn_blocking(move || {
        // Blocking work survives cancellation of its async caller. Keep its
        // slot until the body and DOM are released, including while queued.
        let _permit = parse_permit;
        let body = body;
        let (html, _, _) = encoding.decode(&body);
        parse_content(&html, limit)
    })
    .await
    .map_err(|error| KestrelError::Search(format!("HTML parser task failed: {error}")))?;
    let parse_ms = elapsed_millis(parse_started);
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

fn response_encoding(content_type: &str) -> &'static Encoding {
    content_type
        .parse::<mime::Mime>()
        .ok()
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
    let weighted = extract_weighted_text(root);
    clean_text(&weighted, content_limit)
}

fn remove_page_chrome(document: &mut Html) {
    let mut ids: Vec<_> = document
        .select(&FIXED_CHROME)
        .map(|element| element.id())
        .collect();
    ids.extend(document.select(&CONTAINERS).filter_map(|element| {
        let classes = element
            .value()
            .attr("class")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let id = element
            .value()
            .attr("id")
            .unwrap_or_default()
            .to_ascii_lowercase();
        CLUTTER_PATTERNS
            .iter()
            .any(|pattern| classes.contains(pattern) || id.contains(pattern))
            .then(|| element.id())
    }));
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
    }
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

fn extract_weighted_text(root: ElementRef<'_>) -> String {
    let mut parts = Vec::new();
    for (selector, weight, minimum) in WEIGHTED_TEXT.iter() {
        for element in root.select(selector) {
            let text = element
                .text()
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<String>();
            if text.len() > *minimum {
                parts.extend(std::iter::repeat_n(text, *weight));
            }
        }
    }
    parts.join("\n")
}

fn clean_text(text: &str, content_limit: usize) -> Option<String> {
    let decoded =
        html_escape::decode_html_entities(text).replace(['\u{200b}', '\u{200c}', '\u{200d}'], "");
    let mut deduped = Vec::new();
    let mut previous: Option<&str> = None;
    for line in decoded.lines() {
        if !line.is_empty() && previous != Some(line) {
            deduped.push(line);
        }
        previous = if line.is_empty() { None } else { Some(line) };
    }
    let retained = deduped
        .into_iter()
        .filter(|line| !NOISE.iter().any(|pattern| pattern.is_match(line)))
        .collect::<Vec<_>>()
        .join("\n");
    let cleaned = WHITESPACE.replace_all(&retained, " ").trim().to_owned();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.chars().take(content_limit).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><body><nav>Ignore navigation</nav><aside>Ignore sidebar</aside><main><h1>Kestrel heading</h1><h2>Useful section</h2><p>This is meaningful page content that should be preserved in the extraction.</p><p>Source: ignored metadata</p></main></body></html>"#;

    #[test]
    fn retains_crossing_chunk_prefix_without_exceeding_cap() {
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
                        .set_body_string(page.clone()),
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
                        fetch_one_detailed(&url, &client, network, parsing, &options).await
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
    fn extracts_weighted_main_text_and_noise() {
        let content = parse_content(PAGE, 500).unwrap();
        assert!(content.contains("Kestrel heading"));
        assert!(content.contains("Useful section"));
        assert!(content.contains("meaningful page content"));
        assert!(!content.contains("Ignore navigation"));
        assert!(!content.contains("ignored metadata"));
    }

    #[test]
    fn rejects_documents_without_meaningful_text() {
        assert_eq!(
            parse_content("<html><body><p>short</p></body></html>", 100),
            None
        );
    }

    #[test]
    fn nested_clutter_is_removed() {
        let html = r#"<main><div class="sidebar"><section>Ignored nested clutter</section></div><p>This meaningful content remains available after nested clutter is removed.</p></main>"#;
        assert_eq!(
            parse_content(html, 500).as_deref(),
            Some("This meaningful content remains available after nested clutter is removed.")
        );
    }
}

#[cfg(test)]
mod cutoff_tests;
