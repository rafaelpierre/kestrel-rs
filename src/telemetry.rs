//! Opt-in OpenTelemetry tracing. No global subscriber/provider is installed.
//!
//! Executables explicitly call [`init_from_env`] and [`shutdown`]. Library
//! instrumentation otherwise uses the caller's global OpenTelemetry tracer.
//! See `docs/telemetry.md` for payload policy and subprocess test attribution.
use opentelemetry::{
    Context, KeyValue, global,
    propagation::TextMapPropagator,
    trace::{FutureExt, Status, TraceContextExt, Tracer, TracerProvider},
};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{
        BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracerProvider, SpanData, SpanExporter,
        SpanProcessor,
    },
};
use serde::Serialize;
use std::{
    collections::HashMap,
    future::Future,
    io::Write,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

static RUNTIME: OnceLock<Result<Option<Runtime>, &'static str>> = OnceLock::new();
static ENDED: AtomicUsize = AtomicUsize::new(0);
static EXPORTED: AtomicUsize = AtomicUsize::new(0);
static EXPORT_ERRORS: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone)]
struct PayloadBudget(Arc<AtomicUsize>);
struct Runtime {
    provider: SdkTracerProvider,
    config: Config,
}

/// Validated, bounded export configuration. Secrets are intentionally not Debug.
struct Config {
    endpoint: String,
    protocol: opentelemetry_otlp::Protocol,
    headers: HashMap<String, String>,
    timeout: Duration,
    shutdown: Duration,
    content: bool,
    payload_bytes: usize,
    results: usize,
    sampler: Sampler,
}
fn value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}
fn number(name: &str, default: usize, maximum: usize) -> Result<usize, &'static str> {
    match value(name) {
        None => Ok(default),
        Some(v) => v
            .parse::<usize>()
            .ok()
            .filter(|v| *v > 0 && *v <= maximum)
            .ok_or("invalid telemetry numeric limit"),
    }
}
impl Config {
    fn from_env() -> Result<Option<Self>, &'static str> {
        match value("KESTRELSEARCH_OTEL_ENABLED").as_deref() {
            Some("false" | "0") => return Ok(None),
            None | Some("true" | "1") => {}
            _ => return Err("invalid KESTRELSEARCH_OTEL_ENABLED"),
        }
        let endpoint = if let Some(endpoint) = value("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
            endpoint
        } else if let Some(base) = value("OTEL_EXPORTER_OTLP_ENDPOINT") {
            format!("{}/v1/traces", base.trim_end_matches('/'))
        } else if value("KESTRELSEARCH_OTEL_ENABLED").is_some() {
            return Err("telemetry enabled without an OTLP endpoint");
        } else {
            return Ok(None);
        };
        let url = url::Url::parse(&endpoint).map_err(|_| "invalid OTLP endpoint")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("invalid OTLP endpoint");
        }
        if url.scheme() == "http"
            && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err("OTLP requires HTTPS except on loopback");
        }
        let protocol = match value("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL")
            .or_else(|| value("OTEL_EXPORTER_OTLP_PROTOCOL"))
            .as_deref()
        {
            None | Some("http/protobuf") => opentelemetry_otlp::Protocol::HttpBinary,
            Some("http/json") => opentelemetry_otlp::Protocol::HttpJson,
            _ => return Err("supported OTLP protocols are http/protobuf and http/json"),
        };
        let mut headers = HashMap::new();
        if let Some(raw) = value("OTEL_EXPORTER_OTLP_TRACES_HEADERS")
            .or_else(|| value("OTEL_EXPORTER_OTLP_HEADERS"))
        {
            for entry in raw.split(',') {
                let (key, val) = entry.split_once('=').ok_or("invalid OTLP headers")?;
                if key.trim().is_empty() || val.trim().is_empty() {
                    return Err("invalid OTLP headers");
                }
                let decoded = url::form_urlencoded::parse(
                    format!("v={}", val.trim().replace('+', "%2B")).as_bytes(),
                )
                .next()
                .map(|(_, v)| v.into_owned())
                .ok_or("invalid OTLP headers")?;
                reqwest::header::HeaderName::from_bytes(key.trim().as_bytes())
                    .map_err(|_| "invalid OTLP headers")?;
                reqwest::header::HeaderValue::from_str(&decoded)
                    .map_err(|_| "invalid OTLP headers")?;
                headers.insert(key.trim().to_owned(), decoded);
            }
        }
        let timeout_key = if value("OTEL_EXPORTER_OTLP_TRACES_TIMEOUT").is_some() {
            "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT"
        } else {
            "OTEL_EXPORTER_OTLP_TIMEOUT"
        };
        let content = match value("KESTRELSEARCH_OTEL_CONTENT").as_deref() {
            None | Some("none") => false,
            Some("sanitized") => true,
            _ => return Err("KESTRELSEARCH_OTEL_CONTENT must be none or sanitized"),
        };
        let sampler = match value("OTEL_TRACES_SAMPLER").as_deref() {
            None | Some("parentbased_always_on") => {
                Sampler::ParentBased(Box::new(Sampler::AlwaysOn))
            }
            Some("always_on") => Sampler::AlwaysOn,
            Some("always_off") => Sampler::AlwaysOff,
            Some("traceidratio" | "parentbased_traceidratio") => {
                let ratio = value("OTEL_TRACES_SAMPLER_ARG")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    .ok_or("invalid OTEL_TRACES_SAMPLER_ARG")?;
                let sampler = Sampler::TraceIdRatioBased(ratio);
                if value("OTEL_TRACES_SAMPLER").as_deref() == Some("parentbased_traceidratio") {
                    Sampler::ParentBased(Box::new(sampler))
                } else {
                    sampler
                }
            }
            _ => return Err("unsupported OTEL_TRACES_SAMPLER"),
        };
        Ok(Some(Self {
            endpoint,
            protocol,
            headers,
            timeout: Duration::from_millis(number(timeout_key, 3000, 30000)? as u64),
            shutdown: Duration::from_millis(
                number("KESTRELSEARCH_OTEL_SHUTDOWN_MS", 5000, 30000)? as u64
            ),
            content,
            payload_bytes: number("KESTRELSEARCH_OTEL_PAYLOAD_BYTES", 8192, 65536)?,
            results: number("KESTRELSEARCH_OTEL_RESULT_LIMIT", 20, 100)?,
            sampler,
        }))
    }
}
struct CheckedExporter(opentelemetry_otlp::SpanExporter);
impl std::fmt::Debug for CheckedExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CheckedExporter")
    }
}
impl SpanExporter for CheckedExporter {
    async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
        let count = batch.len();
        let result = self.0.export(batch).await;
        if result.is_ok() {
            EXPORTED.fetch_add(count, Ordering::Relaxed);
        }
        if result.is_err() {
            EXPORT_ERRORS.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "[kestrel] OTLP export failed; check endpoint, credentials and receiver availability"
            );
        }
        result
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }
}

/// Initialize this process's private exporter once, from environment variables.
/// Errors are redacted and never contain endpoint/header values. Does not change
/// the caller's global tracer or subscriber. Call before starting async work.
pub fn init_from_env() -> Result<bool, &'static str> {
    RUNTIME
        .get_or_init(|| {
            let Some(config) = Config::from_env()? else {
                return Ok(None);
            };
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(config.endpoint.clone())
                .with_protocol(config.protocol)
                .with_timeout(config.timeout)
                .with_headers(config.headers.clone())
                .build()
                .map_err(|_| "could not initialize OTLP exporter")?;
            let processor = BatchSpanProcessor::builder(CheckedExporter(exporter))
                .with_batch_config(
                    BatchConfigBuilder::default()
                        .with_max_queue_size(2048)
                        .with_max_export_batch_size(128)
                        .with_scheduled_delay(Duration::from_millis(200))
                        .build(),
                )
                .build();
            let provider = SdkTracerProvider::builder()
                .with_span_processor(CountedProcessor(processor))
                .with_sampler(config.sampler.clone())
                .with_max_events_per_span(128)
                .with_resource(
                    Resource::builder()
                        .with_service_name(
                            value("OTEL_SERVICE_NAME").unwrap_or_else(|| "kestrel".into()),
                        )
                        .with_attributes([KeyValue::new(
                            "service.version",
                            env!("CARGO_PKG_VERSION"),
                        )])
                        .build(),
                )
                .build();
            Ok(Some(Runtime { provider, config }))
        })
        .as_ref()
        .map(|r| r.is_some())
        .map_err(|e| *e)
}
fn runtime() -> Option<&'static Runtime> {
    RUNTIME
        .get()
        .and_then(|r| r.as_ref().ok())
        .and_then(Option::as_ref)
}
/// Flush queued spans at an explicit process/test boundary. Functional outcomes
/// stay separate from the return value, which reports delivery errors.
pub fn flush() -> bool {
    let success = runtime().is_none_or(|r| r.provider.force_flush().is_ok());
    let ok = success
        && EXPORT_ERRORS.load(Ordering::Relaxed) == 0
        && ENDED.load(Ordering::Relaxed) == EXPORTED.load(Ordering::Relaxed);
    receipt(ok);
    ok
}
/// Shut down the private exporter with a bounded wait. Call outside async work.
pub fn shutdown() -> bool {
    let success =
        runtime().is_none_or(|r| r.provider.shutdown_with_timeout(r.config.shutdown).is_ok());
    let ok = success
        && EXPORT_ERRORS.load(Ordering::Relaxed) == 0
        && ENDED.load(Ordering::Relaxed) == EXPORTED.load(Ordering::Relaxed);
    receipt(ok);
    ok
}
/// Environment trace context is used only if the caller has no active span.
pub fn parent_context() -> Context {
    let current = Context::current();
    if current.span().span_context().is_valid() {
        return current;
    }
    let headers = ["traceparent", "tracestate"]
        .into_iter()
        .filter_map(|key| value(&key.to_ascii_uppercase()).map(|v| (key.to_owned(), v)))
        .collect::<HashMap<_, _>>();
    TraceContextPropagator::new().extract(&headers)
}
/// An explicitly owned span. Dropping it closes cancelled/unwound work once.
pub struct Span {
    context: Context,
    complete: bool,
}
impl Span {
    /// Start a span under the current (or environment-propagated) context.
    pub fn new(name: &'static str) -> Self {
        Self::with_parent(name, &parent_context())
    }
    /// Start a child with explicit ownership, including never-polled jobs.
    pub fn with_parent(name: &'static str, parent: &Context) -> Self {
        let context = if let Some(r) = runtime() {
            parent.with_span(
                r.provider
                    .tracer("kestrelsearch")
                    .start_with_context(name, parent),
            )
        } else {
            parent.with_span(global::tracer("kestrelsearch").start_with_context(name, parent))
        };
        let context = context.with_value(PayloadBudget(Arc::new(AtomicUsize::new(65536))));
        context
            .span()
            .set_attribute(KeyValue::new("kestrel.schema_version", 1_i64));
        for (env, key) in [
            ("KESTRELSEARCH_BENCHMARK_RUN_ID", "benchmark.run_id"),
            ("KESTRELSEARCH_OTEL_RUN_ID", "test.run_id"),
            ("KESTRELSEARCH_OTEL_TEST_ID", "test.id"),
        ] {
            if let Some(v) = value(env) {
                context
                    .span()
                    .set_attribute(KeyValue::new(key, v.chars().take(256).collect::<String>()));
            }
        }
        Self {
            context,
            complete: false,
        }
    }
    /// Context for FutureExt::with_context, blocking workers or subprocesses.
    pub fn context(&self) -> Context {
        self.context.clone()
    }
    /// Mark normal completion. Error/cancellation classifications may be added
    /// separately through attributes without conflating partial results.
    pub fn finish(&mut self) {
        self.complete = true;
    }
    /// Set an allowlisted, nonsensitive scalar attribute.
    pub fn attribute(&self, key: &'static str, value: impl Into<opentelemetry::Value>) {
        self.context.span().set_attribute(KeyValue::new(key, value));
    }
    /// Propagate W3C context without mutating process-global environment.
    pub fn inject(&self, command: &mut std::process::Command) {
        let mut headers = HashMap::new();
        TraceContextPropagator::new().inject_context(&self.context, &mut headers);
        for (key, val) in headers {
            command.env(key.to_ascii_uppercase(), val);
        }
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if !self.complete {
            self.context
                .span()
                .set_attribute(KeyValue::new("kestrel.interrupted", true));
        }
        if std::thread::panicking() {
            self.context.span().set_status(Status::error("panic"));
        }
        self.context.span().end();
    }
}
/// Instrument an async operation without holding a thread-local guard over await.
pub async fn scope<T>(name: &'static str, future: impl Future<Output = T>) -> T {
    let mut span = Span::new(name);
    let result = future.with_context(span.context()).await;
    span.finish();
    result
}
/// Instrument synchronous work; do not use this to return an unpolled future.
pub fn scope_sync<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let mut span = Span::new(name);
    let result = {
        let _context = span.context().attach();
        f()
    };
    span.finish();
    result
}
/// Record a nonsensitive status/count on the currently executing operation.
pub fn attribute(key: &'static str, value: impl Into<opentelemetry::Value>) {
    Context::current()
        .span()
        .set_attribute(KeyValue::new(key, value));
}
/// Classify a failure without exporting an arbitrary error message.
pub fn error(kind: &'static str) {
    Context::current().span().set_status(Status::error(kind));
}

struct Limited {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}
impl Write for Limited {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let available = self.limit.saturating_sub(self.bytes.len());
        if buf.len() > available {
            self.bytes.extend_from_slice(&buf[..available]);
            self.truncated = true;
            return Err(std::io::Error::other("telemetry payload limit"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn sanitize(text: &str, secrets: impl Iterator<Item = String>) -> String {
    static URLS: OnceLock<regex::Regex> = OnceLock::new();
    let re = URLS
        .get_or_init(|| regex::Regex::new(r#"https?://[^\s"<>\\]+"#).expect("constant URL regex"));
    let mut text = re
        .replace_all(text, |caps: &regex::Captures<'_>| {
            url::Url::parse(&caps[0])
                .map(|mut u| {
                    let _ = u.set_username("");
                    let _ = u.set_password(None);
                    u.set_query(None);
                    u.set_fragment(None);
                    u.to_string()
                })
                .unwrap_or_else(|_| "[redacted-url]".into())
        })
        .into_owned();
    for secret in secrets.filter(|s| !s.is_empty()) {
        text = text.replace(&secret, "[redacted]");
    }
    text
}
/// Bounded JSON-prefix event. Callers pass only allowlisted fields, never raw
/// HTTP headers, environment maps or provider response bodies. A truncated
/// prefix is explicitly labelled and need not be valid JSON.
pub fn payload<T: Serialize + ?Sized>(stage: &'static str, input: &T) {
    payload_snapshot(stage, input, 0);
}
fn payload_snapshot<T: Serialize + ?Sized>(stage: &'static str, input: &T, omitted: usize) {
    let Some(r) = runtime().filter(|r| r.config.content) else {
        return;
    };
    let current = Context::current();
    if !current.span().is_recording() {
        return;
    }
    let Some(budget) = current.get::<PayloadBudget>() else {
        return;
    };
    let previous = budget
        .0
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
            Some(remaining.saturating_sub(r.config.payload_bytes))
        })
        .unwrap_or(0);
    let reserved = previous.min(r.config.payload_bytes);
    if reserved == 0 {
        current
            .span()
            .set_attribute(KeyValue::new("kestrel.payload.budget_exhausted", true));
        return;
    }
    let mut output = Limited {
        bytes: Vec::new(),
        limit: reserved,
        truncated: false,
    };
    let failed = serde_json::to_writer(&mut output, input).is_err();
    let mut text = sanitize(
        &String::from_utf8_lossy(&output.bytes),
        r.config.headers.values().cloned(),
    );
    let mut truncated = output.truncated;
    if text.len() > reserved {
        let mut end = reserved;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        truncated = true;
    }
    budget
        .0
        .fetch_add(reserved.saturating_sub(text.len()), Ordering::Relaxed);
    current.span().add_event(
        stage,
        vec![
            KeyValue::new("kestrel.payload", text),
            KeyValue::new("kestrel.payload.omitted_results", omitted as i64),
            KeyValue::new("kestrel.payload.truncated", truncated),
            KeyValue::new(
                "kestrel.payload.serialization_error",
                failed && !output.truncated,
            ),
        ],
    );
}
/// Capture a bounded result snapshot while preserving URL/source identity.
pub fn results(stage: &'static str, results: &[crate::SearchResult]) {
    attribute("kestrel.result_count", results.len() as i64);
    if let Some(r) = runtime().filter(|r| r.config.content) {
        let count = results.len().min(r.config.results);
        use sha2::{Digest, Sha256};
        #[derive(Serialize)]
        struct Record<'a> {
            result_id: String,
            #[serde(flatten)]
            result: &'a crate::SearchResult,
        }
        #[derive(Serialize)]
        struct Snapshot<'a> {
            omitted_results: usize,
            results: Vec<Record<'a>>,
        }
        let snapshot = Snapshot {
            omitted_results: results.len() - count,
            results: results[..count]
                .iter()
                .map(|result| Record {
                    result_id: format!(
                        "{:x}",
                        Sha256::digest(crate::search::canonical_url(&result.url).as_bytes())
                    ),
                    result,
                })
                .collect(),
        };
        payload_snapshot(stage, &snapshot, results.len() - count);
    }
}
/// Test-only lifecycle guard. Tests are attributed by the subprocess runner,
/// avoiding unsafe environment changes or context leakage across async tests.
#[doc(hidden)]
pub fn test_export_guard() -> TestExportGuard {
    if let Err(message) = init_from_env() {
        eprintln!("[kestrel] {message}");
    }
    TestExportGuard
}
#[doc(hidden)]
pub struct TestExportGuard;
impl Drop for TestExportGuard {
    fn drop(&mut self) {
        if !flush() {
            eprintln!("[kestrel] test telemetry delivery failed");
        }
    }
}

/// Capture synchronous ranking output under the ranking span.
pub fn scope_results(
    name: &'static str,
    f: impl FnOnce() -> Vec<crate::SearchResult>,
) -> Vec<crate::SearchResult> {
    scope_sync(name, || {
        let output = f();
        results("ranking.output", &output);
        output
    })
}

#[derive(Debug)]
struct CountedProcessor(BatchSpanProcessor);
impl SpanProcessor for CountedProcessor {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &Context) {
        self.0.on_start(span, cx);
    }
    fn on_end(&self, span: SpanData) {
        if span.span_context.is_sampled() {
            ENDED.fetch_add(1, Ordering::Relaxed);
        }
        self.0.on_end(span);
    }
    fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.force_flush()
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}
fn receipt(ok: bool) {
    if runtime().is_some()
        && let Some(dir) = value("KESTRELSEARCH_OTEL_RECEIPT_DIR")
    {
        let path = std::path::Path::new(&dir).join(std::process::id().to_string());
        // A failed receipt is sticky for the lifetime of the process.
        if (!ok || std::fs::read_to_string(&path).ok().as_deref() != Some("failed"))
            && std::fs::write(path, if ok { "ok" } else { "failed" }).is_err()
        {
            eprintln!("[kestrel] could not persist telemetry delivery receipt");
        }
    }
}

/// Async Result instrumentation records failure status without leaking errors.
pub async fn scope_result<T, E>(
    name: &'static str,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    scope(name, async {
        let output = future.await;
        if output.is_err() {
            error("operation_failed");
        }
        output
    })
    .await
}
/// CLI exit status is preserved and also reflected in the command span.
pub async fn scope_exit(
    name: &'static str,
    future: impl Future<Output = std::process::ExitCode>,
) -> std::process::ExitCode {
    scope(name, async {
        let output = future.await;
        if output != std::process::ExitCode::SUCCESS {
            error("command_failed");
        }
        output
    })
    .await
}
