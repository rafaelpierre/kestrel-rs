# OpenTelemetry and Honeycomb

Kestrel uses the OpenTelemetry Rust API, SDK and OTLP exporter. Honeycomb's
[official Rust guidance](https://docs.honeycomb.io/send-data/opentelemetry)
recommends `opentelemetry` and `opentelemetry-otlp`; this integration does not use
an unofficial Honeycomb-specific tracing library. The three 0.32 crates support
Rust 1.75 or newer; Kestrel retains its Rust 1.89 minimum. No tracing subscriber
is installed or replaced. Metrics and log export are outside this integration.

## Configuration

The CLI explicitly initializes a private exporter before starting its async
runtime. Without an OTLP endpoint it does not export remotely. Set an endpoint
to enable export, or explicitly disable it with `KESTRELSEARCH_OTEL_ENABLED=false`.
Invalid telemetry settings emit a redacted stderr diagnostic and disable export;
functional CLI output and exit status remain independent of export failures.
The test runner rejects invalid export setup instead of reporting delivery success.

```sh
# Choose the endpoint matching your Honeycomb account's region:
export OTEL_EXPORTER_OTLP_ENDPOINT=https://api.eu1.honeycomb.io
# US: https://api.honeycomb.io
export OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=$HONEYCOMB_API_KEY"
export OTEL_SERVICE_NAME=kestrel-tests
export OTEL_TRACES_SAMPLER=always_on
export KESTRELSEARCH_OTEL_CONTENT=sanitized
python3 scripts/test_traces.py
```

Provision `HONEYCOMB_API_KEY` through your secret manager/shell environment.
Kestrel reads the OTLP headers variable, not `HONEYCOMB_API_KEY` directly.
Honeycomb Classic routing can additionally use `x-honeycomb-dataset=DATASET` in
the comma-separated headers. Confirm routing and the region for your account.
An exporter acknowledgement is not proof of queryable Honeycomb ingestion:
confirm the run ID, test counts, trace hierarchy and content in Honeycomb.

| Variable | Default / supported behavior |
| --- | --- |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | No default remote destination; appends `/v1/traces` |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Full trace URL, overrides general endpoint |
| `OTEL_EXPORTER_OTLP_PROTOCOL`, `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` | `http/protobuf`; also `http/json`; trace-specific value wins; gRPC rejected |
| `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_EXPORTER_OTLP_TRACES_HEADERS` | Comma-separated `key=value` headers, percent-decoded; trace-specific value wins |
| `OTEL_EXPORTER_OTLP_TIMEOUT`, `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` | 3000 milliseconds; 1–30000; trace-specific value wins |
| `OTEL_SERVICE_NAME` | `kestrel` |
| `OTEL_RESOURCE_ATTRIBUTES` | SDK comma-separated resource attributes; set revision, environment and CI run identity here |
| `OTEL_TRACES_SAMPLER` | `parentbased_always_on`; also `always_on`, `always_off`, `traceidratio`, `parentbased_traceidratio` |
| `OTEL_TRACES_SAMPLER_ARG` | Required finite 0–1 ratio for ratio samplers |
| `KESTRELSEARCH_OTEL_ENABLED` | Endpoint enables export; `true`/`1` requires an endpoint; `false`/`0` disables it |
| `KESTRELSEARCH_OTEL_CONTENT` | `none` or opt-in `sanitized` |
| `KESTRELSEARCH_OTEL_PAYLOAD_BYTES` | 8192 bytes per event, 1–65536 |
| `KESTRELSEARCH_OTEL_RESULT_LIMIT` | First 20 results per snapshot, 1–100; omitted count included |
| `KESTRELSEARCH_OTEL_SHUTDOWN_MS` | 5000 milliseconds, 1–30000 |
| `KESTRELSEARCH_OTEL_RUN_ID`, `KESTRELSEARCH_OTEL_TEST_ID` | Optional test correlation; runner supplies them |
| `KESTRELSEARCH_BENCHMARK_RUN_ID` | Existing benchmark correlation, preserved |
| `TRACEPARENT`, `TRACESTATE` | W3C parent context for subprocesses; active caller context takes precedence |

HTTPS is required except for loopback receivers. Endpoint credentials, query
strings and fragments are rejected. Batch export uses a dedicated SDK worker,
a 2048-span queue, batches of 128, and a 200 ms scheduled delay. These queue and
batch bounds are explicit, not unbounded tasks per event. No additional exporter
retry loop is enabled. A failed batch increments an error counter; comparing ended
sampled spans with exported spans at flush/shutdown detects loss, including queue
overflow. This is local accounting, not backend retention/sampling verification.
The SDK limits events to 128 per span and reports dropped events in OTLP.
A shared 64 KiB content budget per span additionally bounds aggregate payload
memory; exhaustion sets `kestrel.payload.budget_exhausted` and suppresses later
payloads on that span. Child spans have independent budgets.

## Hierarchy and payloads

```text
test.run
  Rust executable / Python test suite
    test case (or explicit ignored-test skip)
      CLI search / fetch
        search / query / provider
          queue, HTTP attempt / send / body / parse, backoff
        merge and selected candidates
        fetch / page / HTTP attempt / send / body
          extraction and content-quality assessment
        ranking
```

`kestrel.schema_version=1` identifies instrumentation. Named events record queries,
provider raw/accepted streaming snapshots, rejected records, provider results,
merged results, selected candidates, cache contents, page contents, quality,
ranking inputs/outputs and final CLI results. URL and source fields retain result
provenance between snapshots; a SHA-256 `result_id` derived from the canonical URL joins sanitized snapshots. Logical provider spans include completion/cancellation
outcomes; attempt spans retain run/search/attempt IDs, ordinal, known HTTP status,
challenge and typed transport error. Phase spans mark censored work. Application
sends are counted, not hidden redirect hops, DNS, TCP or TLS timing.

Content is disabled by default. `sanitized` enables **allowlisted** serialized
inputs, outputs and intermediate results, not arbitrary response bodies/headers or
environment maps. URL userinfo, all query strings and fragments are removed from
payloads; configured OTLP header values are redacted. It is not a general-purpose
PII detector: use sanitized fixtures and do not enable content capture for private
queries/pages without an appropriate data policy. Result payloads may still contain
public query text and extracted prose. Export credentials are never diagnostic
attributes. A bounded serialization writer stops at the payload limit; a truncated
payload is explicitly a JSON prefix and may not be parseable JSON. The final
sanitized UTF-8 string is bounded again. `kestrel.payload.truncated` and snapshot
`omitted_results` disclose limits. SDK event drops are separate from payload limits.

Async scopes restore context on each future poll. Blocking extraction/parser work
explicitly carries parent context. Provider guards create spans before polling and
finalize cancelled or never-polled work. A blocking parser can finish after its
cancelled parent; this reflects the existing cancellation semantics. Public JSON,
provider ordering, budgets, diagnostic files and benchmark schemas are unchanged.

## Tests and benchmarks

`python3 scripts/test_traces.py` builds and lists all Rust test executables, then
runs each selected test in its own process under a run/suite/test hierarchy.
Every existing Rust test initializes/flushes the exporter using a small guard.
The runner owns pass/fail/skip classification, including expected-panic tests;
no thread-local guard is held across async test awaits. Child CLI commands inherit
the test context. Export-disabled ordinary `cargo test --all-features` remains the
fast parallel correctness/concurrency check. The traced runner deliberately changes
process isolation and must not replace that concurrency check or be treated as a
performance benchmark of the normal harness.

| Entry point | Traced execution |
| --- | --- |
| Rust unit, integration and CLI tests | Automatically enumerated, one span per executed test; guard flushes child spans |
| Ignored/live Rust tests | Skip spans by default; explicitly run with `--include-ignored` |
| Python benchmark contract tests | Discover each unittest in `benchmarks` and `benchmarks/budget-overhead`, execute per test |
| Rust doc tests | Cargo doc-test suite span; currently no executable doc tests in this repository |
| Controlled/live benchmark scripts | Wrap each scenario/repetition with `target/debug/examples/trace_command NAME COMMAND ARGS...`; existing benchmark run IDs also correlate search spans |
| q01–q10 evidence workflow | Wrap each question's search/fetch command with `trace_command qNN ...`; preserve the exact manifest, budgets, evidence and grading required by `AGENTS.md` |

`--filter TEXT` limits the runner; it does not claim full coverage. The run ID is
inherited through all processes. The helper reports process outcomes separately
from telemetry receipt failures (exit 3). Receipt files in temporary directories
contain only `ok`/`failed` and are removed at exit. Child processes that never
initialize telemetry have no receipt; parent test spans still record their exit.
Abrupt process kill can lose in-flight spans and receipts; no delivery guarantee
is claimed for that case.

```sh
# Fully local delivery/hierarchy check; keeps raw telemetry in memory:
python3 scripts/verify_test_traces.py --summary /tmp/kestrel-traces-summary.json
# Quick provider fixture check:
python3 scripts/verify_test_traces.py --filter filtered_results_record
# Instrument an existing benchmark without changing its CLI arguments:
cargo build --example trace_command
target/debug/examples/trace_command benchmark.scenario python3 benchmarks/quality_latency.py --help
```

CI retains the ordinary checks, then runs traced tests if the repository has the
`HONEYCOMB_API_KEY` secret and `HONEYCOMB_OTLP_ENDPOINT` variable. Secretless/fork
runs explicitly report export unavailable. All test traces use `always_on` and
sanitized content; receiver-side sampling/retention must be verified separately.
Do not set secrets in PR text, checked-in files or captured shell output.

## Library lifecycle

Call `telemetry::init_from_env()` explicitly at your application's startup and
`telemetry::shutdown()` outside async work on exit to use Kestrel's private exporter.
Alternatively install your own OpenTelemetry global tracer and propagate its
`Context`: Kestrel uses it when no private exporter is configured, without replacing
it. Payload capture is only enabled through the explicit private configuration.
The library never auto-initializes or shuts down your global provider. Use
`FutureExt::with_context` for tasks you spawn yourself. `telemetry::flush()` is an
explicit blocking boundary (SDK bounded wait); tests flush at their end, not inside
request/drop paths. Native `tracing` subscribers need their own OpenTelemetry bridge.

## Validation and remaining external evidence

Local wire tests assert fetch ancestry, inherited parent IDs, payload bounds,
redaction, disabled/metadata modes and failed exporter behavior. The traced runner's
loopback check validates process ancestry. Run fmt, clippy, all-feature tests,
release build and the required evidence gate before claiming readiness. Measure
disabled/enabled overhead following `benchmarks/README.md`; no latency improvement
or negligible-overhead claim follows from instrumentation tests. Actual Honeycomb
ingestion and a complete credentialed evidence run require a configured account.

## HTTP request headers

Each provider send attempt and page fetch attempt exports `user_agent.original`
and `http.request.header.user_agent`, plus present allowlisted headers under
`http.request.header.*`: `accept`, `accept_language`, `accept_encoding`,
`content_type`, `sec_ch_ua`, `sec_ch_ua_mobile`, `sec_ch_ua_platform`,
`sec_fetch_dest`, `sec_fetch_mode`, `sec_fetch_site`, `sec_fetch_user`, and
`upgrade_insecure_requests`. Values are limited to 512 Unicode characters each;
non-text and explicitly sensitive values are omitted. These metadata attributes
are available with content capture disabled and use the existing OTLP/Honeycomb
export configuration. No exporter endpoint means no remote storage.

Generated defaults remain attached to pooled clients and provider overrides win.
The attributes describe the initial application request, including failed sends
and retries; they do not claim to capture redirected requests or headers added
by the transport or a proxy. Cookies, authorization, arbitrary headers, Origin,
Referer and response headers are never included in this allowlist. Browser
profiles are restricted to Firefox 146/Windows and Chrome 146/macOS.
