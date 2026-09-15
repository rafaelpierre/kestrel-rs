//! Provider HTTP execution, bounded body reads and existing backend retry policies.
use super::{
    ProviderFailure, discovery, observe, parsing, record_attempt, record_headers, record_phase,
    streaming,
};
use crate::{
    error::KestrelError,
    model::Engine,
    provider_diagnostics::{Challenge, Phase, TransportKind},
    providers::response::{ParsedResponse, process_completed},
};
use futures_util::StreamExt;
#[cfg(test)]
use std::sync::Arc;
use std::{future::Future, time::Duration};
pub(crate) fn is_tls_error(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    for _ in 0..64 {
        let Some(error) = source else { break };
        if error.is::<rustls::Error>() || error.is::<primp_tls::Error>() {
            return true;
        }
        // io::Error::source skips the wrapped error itself. Inspect get_ref()
        // first so a TLS error directly wrapped by the transport is not lost.
        source = error
            .downcast_ref::<std::io::Error>()
            .and_then(|io| {
                io.get_ref()
                    .map(|inner| inner as &(dyn std::error::Error + 'static))
            })
            .or_else(|| error.source());
    }
    false
}

fn standard_transport(error: &reqwest::Error) -> TransportKind {
    if error.is_timeout() {
        TransportKind::Timeout
    } else if is_tls_error(error) {
        TransportKind::Tls
    } else if error.is_connect() {
        TransportKind::Connect
    } else if error.is_decode() {
        TransportKind::Decode
    } else if error.is_body() {
        TransportKind::Body
    } else if error.is_request() {
        TransportKind::Request
    } else {
        TransportKind::Unknown
    }
}

fn yahoo_transport(error: &primp::Error) -> TransportKind {
    if error.is_timeout() {
        TransportKind::Timeout
    } else if is_tls_error(error) {
        TransportKind::Tls
    } else if error.is_dns() {
        TransportKind::Dns
    } else if error.is_connect() {
        TransportKind::Connect
    } else if error.is_decode() {
        TransportKind::Decode
    } else if error.is_body() {
        TransportKind::Body
    } else if error.is_request() {
        TransportKind::Request
    } else {
        TransportKind::Unknown
    }
}

pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum decompressed response bytes accepted from any search provider.
/// Independent of page-fetch limits; applies to success and HTTP error bodies.
pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
// Keep backend-specific send-error eligibility explicit. Completed responses and
// body failures use the same policy regardless of the HTTP implementation.
struct SendFailure {
    timeout: bool,
    retryable: bool,
    kind: TransportKind,
    error: KestrelError,
}

enum ProviderHttpResponse {
    Standard(reqwest::Response),
    Impersonated {
        engine: Engine,
        response: primp::Response,
    },
}

struct ResponseHead {
    status: u16,
    retry_after: Option<String>,
    final_url: String,
    http_version: String,
}

impl ProviderHttpResponse {
    fn head(&self) -> ResponseHead {
        match self {
            Self::Standard(response) => ResponseHead {
                status: response.status().as_u16(),
                retry_after: response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
                final_url: response.url().to_string(),
                http_version: format!("{:?}", response.version()),
            },
            Self::Impersonated { response, .. } => ResponseHead {
                status: response.status().as_u16(),
                retry_after: response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
                final_url: response.url().to_string(),
                http_version: format!("{:?}", response.version()),
            },
        }
    }

    async fn read(self, engine: Engine) -> Result<String, KestrelError> {
        match self {
            Self::Standard(response) => read_standard_body(response, engine).await,
            Self::Impersonated { engine, response } => {
                read_impersonated_body(response, engine).await
            }
        }
    }
}

fn impersonated_error(engine: Engine, error: primp::Error) -> KestrelError {
    match engine {
        Engine::Yahoo => KestrelError::Yahoo(error),
        Engine::Bing => KestrelError::Bing(error),
        _ => KestrelError::Search(format!("{engine} impersonated request failed: {error}")),
    }
}

pub(crate) async fn request_yahoo_with_retries<F, T: Send + 'static>(
    query: &str,
    extract: fn(Engine, &str, &ParsedResponse) -> T,
    build: F,
) -> Result<(T, usize), ProviderFailure>
where
    F: Fn() -> primp::RequestBuilder,
{
    request_impersonated_with_retries(Engine::Yahoo, query, extract, build).await
}

pub(crate) async fn request_impersonated_with_retries<F, T: Send + 'static>(
    engine: Engine,
    query: &str,
    extract: fn(Engine, &str, &ParsedResponse) -> T,
    build: F,
) -> Result<(T, usize), ProviderFailure>
where
    F: Fn() -> primp::RequestBuilder,
{
    request_with_retries(engine, query, extract, || async {
        let request = build();
        if let Some(copy) = request.try_clone() {
            let (client, built) = copy.build_split();
            if let Ok(built) = built {
                let mut headers = client.headers().clone();
                headers.extend(built.headers().clone());
                observe(|recorder| recorder.request_headers(&headers));
            }
        }
        request
            .send()
            .await
            .map(|response| ProviderHttpResponse::Impersonated { engine, response })
            .map_err(|error| SendFailure {
                timeout: error.is_timeout(),
                retryable: true,
                kind: yahoo_transport(&error),
                error: impersonated_error(engine, error),
            })
    })
    .await
}

pub(crate) async fn request_standard_with_retries<F, T: Send + 'static>(
    client: &crate::http_client::Client,
    engine: Engine,
    query: &str,
    extract: fn(Engine, &str, &ParsedResponse) -> T,
    build: F,
) -> Result<(T, usize), ProviderFailure>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    request_with_retries(engine, query, extract, || async {
        let request = build();
        if let Some(copy) = request.try_clone()
            && let Ok(built) = copy.build()
        {
            observe(|recorder| recorder.request_headers(&client.request_headers(built.headers())));
        }
        request
            .send()
            .await
            .map(ProviderHttpResponse::Standard)
            .map_err(|error| SendFailure {
                timeout: error.is_timeout(),
                retryable: error.is_timeout() || error.is_connect() || error.is_request(),
                kind: standard_transport(&error),
                error: error.into(),
            })
    })
    .await
}

async fn request_with_retries<F, Fut, T: Send + 'static>(
    engine: Engine,
    query: &str,
    extract: fn(Engine, &str, &ParsedResponse) -> T,
    send: F,
) -> Result<(T, usize), ProviderFailure>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<ProviderHttpResponse, SendFailure>>,
{
    let mut last_error = None;
    for attempt in 1..=3 {
        let mut server_delay = None;
        record_attempt();
        let response = send().await;
        observe(|r| {
            r.transition_censored(
                Phase::Processing,
                response.as_ref().err().is_some_and(|e| e.timeout),
            )
        });
        match response {
            Ok(response) => {
                let ResponseHead {
                    status,
                    retry_after,
                    final_url,
                    http_version,
                } = response.head();
                server_delay = retry_after.as_deref().and_then(discovery::retry_after);
                record_headers(status, retry_after);
                record_phase(Phase::Body);
                let body = response.read(engine).await;
                observe(|r| {
                    r.transition_censored(
                        Phase::Processing,
                        body.as_ref().err().is_some_and(body_read_censored),
                    )
                });
                match body {
                    Ok(html) => {
                        record_phase(Phase::Parse);
                        let (html, challenge, results) = parsing::run(move || {
                            let (challenge, results) = process_completed(
                                engine,
                                &html,
                                (200..300).contains(&status),
                                extract,
                            );
                            (html, challenge, results)
                        })
                        .await?;
                        observe(|r| r.response(challenge));
                        record_phase(Phase::Processing);
                        crate::benchmarking::capture_provider(
                            engine,
                            query,
                            &final_url,
                            status,
                            &http_version,
                            attempt,
                            &html,
                        );
                        if challenge == Challenge::Detected && !(200..300).contains(&status) {
                            return Err(ProviderFailure::challenge(format!(
                                "{engine} returned a bot challenge (HTTP {status})"
                            )));
                        }
                        if (200..300).contains(&status) {
                            record_phase(Phase::Parse);
                            return results.map(|results| (results, attempt - 1)).ok_or_else(
                                || {
                                    KestrelError::Search(
                                        "successful response missing extraction".into(),
                                    )
                                    .into()
                                },
                            );
                        }
                    }
                    Err(error) => {
                        record_body_error(&error);
                        // Preserve the successful-status body failure policy. For error
                        // statuses retain status-based retries, while keeping the body error.
                        if (200..300).contains(&status)
                            || matches!(error, KestrelError::ProviderResponseTooLarge { .. })
                        {
                            return Err(error.into());
                        }
                    }
                }
                let retryable = status == 408 || status == 429 || status >= 500;
                let error = KestrelError::Search(format!("{engine} returned HTTP {status}"));
                if !retryable || attempt == 3 {
                    return Err(error.into());
                }
                last_error = Some(error);
            }
            Err(error) => {
                observe(|r| r.error(error.kind, false));
                if !error.retryable || attempt == 3 {
                    return Err(error.error.into());
                }
                last_error = Some(error.error);
            }
        }
        log_retry(engine, query, attempt, last_error.as_ref());
        if let Some(delay) = server_delay {
            // Do not shorten server guidance or permit an unbounded wait when
            // callers disable the discovery deadline. End this request instead.
            if delay > SEARCH_TIMEOUT {
                return Err(last_error
                    .unwrap_or_else(|| {
                        KestrelError::Search(
                            "server retry delay exceeds 15s request retry allowance".into(),
                        )
                    })
                    .into());
            }
            record_phase(Phase::Backoff);
            tokio::time::sleep(delay).await;
        } else {
            retry_delay(attempt).await;
        }
    }
    Err(last_error
        .unwrap_or_else(|| KestrelError::Search("request failed".into()))
        .into())
}

fn body_read_censored(error: &KestrelError) -> bool {
    match error {
        KestrelError::Http(error) => error.is_timeout(),
        KestrelError::Yahoo(error) => error.is_timeout(),
        KestrelError::Bing(error) => error.is_timeout(),
        KestrelError::ProviderResponseTooLarge { .. } => true,
        _ => false,
    }
}

fn record_body_error(error: &KestrelError) {
    observe(|recorder| match error {
        KestrelError::Http(error) => recorder.error(standard_transport(error), true),
        KestrelError::Yahoo(error) => recorder.error(yahoo_transport(error), true),
        KestrelError::Bing(error) => recorder.error(yahoo_transport(error), true),
        KestrelError::ProviderResponseTooLarge { .. } => recorder.response_too_large(),
        _ => recorder.error(TransportKind::Unknown, true),
    });
}

// Both transports expose decompressed chunks. Check before appending, including
// when Content-Length is absent (chunked transfer or automatic decompression).
pub(crate) async fn read_standard_body(
    response: reqwest::Response,
    engine: Engine,
) -> Result<String, KestrelError> {
    let body = ProviderBody::new(
        engine,
        response.status().as_u16(),
        response.content_length(),
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
    )?;
    #[cfg(test)]
    streaming::probe::headers(
        body.engine,
        format!("{:?}", response.version()),
        body.status,
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
    );
    let chunks = futures_util::stream::try_unfold(response, |mut response| async {
        response
            .chunk()
            .await
            .map(|chunk| chunk.map(|chunk| (chunk, response)))
            .map_err(KestrelError::from)
    });
    read_provider_chunks(body, chunks).await
}

async fn read_impersonated_body(
    response: primp::Response,
    engine: Engine,
) -> Result<String, KestrelError> {
    let body = ProviderBody::new(
        engine,
        response.status().as_u16(),
        response.content_length(),
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
    )?;
    #[cfg(test)]
    streaming::probe::headers(
        body.engine,
        format!("{:?}", response.version()),
        body.status,
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
    );
    let chunks = futures_util::stream::try_unfold(response, |mut response| async {
        response
            .chunk()
            .await
            .map(|chunk| chunk.map(|chunk| (chunk, response)))
            .map_err(|error| impersonated_error(engine, error))
    });
    read_provider_chunks(body, chunks).await
}

async fn read_provider_chunks<S, B>(
    mut body: ProviderBody,
    chunks: S,
) -> Result<String, KestrelError>
where
    S: futures_util::Stream<Item = Result<B, KestrelError>>,
    B: AsRef<[u8]>,
{
    let mut incremental = streaming::Incremental::for_body(&body);
    let mut chunks = std::pin::pin!(chunks);
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        let chunk = chunk.as_ref();
        body.push(chunk)?;
        #[cfg(test)]
        streaming::probe::bytes(body.engine, chunk.len());
        if let Some(parser) = &mut incremental {
            parser.push(chunk).await?;
        }
    }
    #[cfg(test)]
    streaming::probe::eof(body.engine);
    // Release the persistent stream worker before waiting for completed-body capacity.
    drop(incremental);
    parsing::run(move || body.text()).await
}

pub(crate) struct ProviderBody {
    pub(super) bytes: Vec<u8>,
    pub(super) encoding: &'static encoding_rs::Encoding,
    pub(super) engine: Engine,
    pub(super) status: u16,
}

impl ProviderBody {
    pub(super) fn new(
        engine: Engine,
        status: u16,
        content_length: Option<u64>,
        content_type: Option<&str>,
    ) -> Result<Self, KestrelError> {
        let mime = content_type.and_then(|value| value.parse::<mime::Mime>().ok());
        let encoding = mime
            .as_ref()
            .and_then(|mime| mime.get_param("charset"))
            .and_then(|charset| encoding_rs::Encoding::for_label(charset.as_str().as_bytes()))
            .unwrap_or(encoding_rs::UTF_8);
        let body = Self {
            bytes: Vec::new(),
            encoding,
            engine,
            status,
        };
        if content_length.is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64) {
            return Err(body.too_large());
        }
        Ok(body)
    }

    fn too_large(&self) -> KestrelError {
        KestrelError::ProviderResponseTooLarge {
            engine: self.engine,
            limit_bytes: MAX_PROVIDER_RESPONSE_BYTES,
            status: self.status,
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<(), KestrelError> {
        if chunk.len() > MAX_PROVIDER_RESPONSE_BYTES - self.bytes.len() {
            return Err(self.too_large());
        }
        let required = self.bytes.len() + chunk.len();
        if required > self.bytes.capacity() {
            // Retain amortized growth without asking Vec to grow past the cap.
            let capacity = required
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_PROVIDER_RESPONSE_BYTES);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    pub(super) fn text(self) -> String {
        // Match Response::text's charset/BOM handling and replacement semantics.
        // UTF-8 expansion and parser allocations remain proportional to the cap.
        self.encoding.decode(&self.bytes).0.into_owned()
    }
}

fn log_retry(engine: Engine, query: &str, attempt: usize, error: Option<&KestrelError>) {
    crate::log_event!(
        "search_retry",
        "engine" => engine.as_str(),
        "query" => query,
        "attempt" => attempt,
        "error_type" => "request",
        "error" => error.map(ToString::to_string).unwrap_or_default(),
    );
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_RETRY_DELAY: Duration;
    pub(crate) static TEST_BACKOFF_ENTERED: Arc<tokio::sync::Notify>;
}

async fn retry_delay(attempt: usize) {
    record_phase(Phase::Backoff);
    #[cfg(test)]
    let _ = TEST_BACKOFF_ENTERED.try_with(|notify| notify.notify_one());
    #[cfg(test)]
    if let Ok(delay) = TEST_RETRY_DELAY.try_with(|delay| *delay) {
        tokio::time::sleep(delay).await;
        return;
    }
    #[cfg(test)]
    if attempt > 0 {
        return;
    }
    let base_ms = (250_u64 * 2_u64.pow((attempt.saturating_sub(1)) as u32)).min(2_000);
    let jitter_ms = (rand::random::<f64>() * base_ms as f64) as u64;
    tokio::time::sleep(Duration::from_millis((base_ms + jitter_ms).min(2_000))).await;
}
