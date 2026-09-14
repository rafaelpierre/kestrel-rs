# Standalone HProxy discovery

Issue [#102](https://github.com/rafaelpierre/kestrel-rs/issues/102) adds the
independent discovery adapter for [#9](https://github.com/rafaelpierre/kestrel-rs/issues/9).
Rotation, health, production routing and CLI initialization remain in #103–#105.
Constructing a Kestrel client, running help, installing skills, searching or
fetching does not invoke this adapter. There are no automatic-proxy CLI flags.

```rust,no_run
use kestrelsearch::proxy::hproxy::{DiscoveryOptions, HProxyDiscovery, Protocol};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let discovery = HProxyDiscovery::new(DiscoveryOptions {
    protocols: vec![Protocol::Https], // advertised HTTPS destination/CONNECT support
    country: Some("GB".into()),
    min_uptime_pct: Some(90.0),
    max_latency_ms: Some(1000),
    ..Default::default()
})?;
let shared = discovery.clone();
let report = shared.discover().await?;
for endpoint in &report.endpoints {
    println!("{} {:?}", endpoint.proxy_url(), endpoint.protocols);
}
# Ok(())
# }
```

`new` builds a separate bootstrap pool using Kestrel's transport policy and
existing environment/system proxy settings. It performs no network lookup.
`with_builder(endpoint, reqwest::ClientBuilder, options)` permits an explicit
bootstrap proxy, trust roots or a loopback fixture. It rejects endpoint
credentials, query strings and fragments. Both constructors disable redirects
and reqwest's implicit retry policy so requests cannot bypass attempt limits.
TLS certificate verification stays enabled by default. Discovery never installs
or rotates an egress proxy or sends requests through discovered endpoints.

## Wire contract and filtering

On 2026-09-14 the official [API documentation](https://hproxy.com/docs/free-proxy-list)
was rechecked through web retrieval: one keyless GET returns a JSON array when
`format=json`; omit `limit`/`offset` for a full matching list. The adapter also
sends `sort=uptime`, comma-separated `protocol`, and configured country/uptime/
latency filters. It omits `recent` unless `include_recent=true`. The documentation
specifies `status=alive` or `recently_alive` and nullable enrichment.

Direct API requests from the implementation host failed with connection resets;
no successful live API-response or public-proxy-health test is claimed. The
[official GitHub snapshot](https://github.com/hproxy-com/free-proxy-list) linked
from HProxy's free-list page was inspected separately. That export has
`protocols` arrays and `country`, but uses an `alive` boolean instead of the API's
documented `status`. It is not an interchangeable endpoint for this adapter.
Tests use synthetic reserved IP addresses, not captured public proxy addresses.

Each API row requires a numeric valid IP, nonzero `u16` port, string `protocols`
array and `status`. Unspecified/multicast addresses and malformed records are
skipped. Unknown enrichment is ignored; nullable `country_code` (also accepted
as `country`), `latency_ms` and `uptime_pct` are retained. Country filters use
case-insensitive two-letter codes, uptime accepts 0–100, and maximum latency
accepts 1–120000 milliseconds. Missing enrichment fails an active filter.
Unknown status is excluded; recently alive rows require explicit opt-in.

Only `Protocol::Http` and `Protocol::Https` can be selected. String parsing
explicitly rejects SOCKS4/SOCKS5 and other unsupported protocols. Mixed rows
retain HTTP/HTTPS capabilities and drop unsupported labels; entirely unsupported
rows are counted and excluded. Protocol selection matches any requested
capability, retaining all supported capabilities of a qualifying row. Endpoints
are deduplicated by normalized IP and port, preserving first eligible occurrence
order and enrichment, and merging supported capabilities from duplicates.

`proxy_url()` uses **http://** for both labels. `Https` means a CONNECT tunnel to
an HTTPS destination, not a TLS connection to the proxy. The caller must inspect
advertised capabilities for its destination; discovery is not a health check.
Local fixtures prove HTTP absolute-form forwarding and CONNECT with validated
TLS certificates for both reqwest and primp. No SOCKS transport support is claimed.

## Resource and failure policy

| Setting | Default | Behavior |
| --- | --- | --- |
| `max_response_bytes` | 16 MiB | Aggregate decoded bytes across successful pages; exact cap accepted only after EOF |
| `max_entries` | 50,000 | Distinct eligible endpoints; exceeding the cap fails, never silently truncates |
| `page_size` | None | One full-list GET; optional explicit page size 1–10000 |
| `max_pages` | 1 | Positive bound; must be 1 without explicit pagination |
| `max_attempts` | 3 | Total requests including pages, 429/5xx and connection/header timeout retries |
| `deadline` | 30 seconds | Entire initialization including throttling, I/O and parsing |
| `request_timeout` | 10 seconds | Individual request including body consumption |
| `min_attempt_interval` | 500 milliseconds | Minimum start-to-start spacing; values below 500ms rejected |

Every size/count is positive and durations must fit a monotonic deadline. Explicit
pagination needs both `X-Total-Available` and `X-Total-Count` on every page.
Counts must match decoded rows, total must remain stable, and offset advances by
raw rows including rejected/duplicate rows. Missing or contradictory counters,
stalled pages or exhausted page limits fail as `IncompleteList`. A full-list
response with headers advertising missing rows also fails. HTTP 206 is rejected.
Byte or entry overflow never yields partial success. JSON parsing runs on one
bounded blocking worker per adapter; cancellation does not release its capacity
before the worker finishes. Already buffered transport data and JSON allocation
overhead are outside the retained body-byte count. Parsing is byte bounded;
runtime shutdown may wait for a cancelled blocking parser.

Clones coalesce discovery and cache the same `Arc<DiscoveryReport>` or typed
`DiscoveryError`, including empty/all-rejected responses. There is no background
refresh. Create a new adapter for another initialization. Cancellation leaves
already consumed attempts, the original deadline, and any recorded Retry-After
intact. Transport failures before headers and 429/5xx may retry; malformed JSON,
body-read errors, other statuses and resource-limit failures do not retry.

`Retry-After` accepts delta seconds or an HTTP date (past dates mean no additional
wait), on both 429 and 5xx. Throttle spacing still applies. Invalid values or a
wait exceeding the remaining deadline return `RetryAfter`; they are not shortened
to permit an early retry. Shared adapters use no burst allowance, at most 120
attempts per minute. Separate adapters/processes and other users behind the same
IP are not globally coordinated; reuse the same adapter and honor service 429s.
The bootstrap route never rotates to evade the service limit.

Successful reports expose pages, attempts, decoded bytes and separate malformed,
unsupported, filtered and duplicate row counts. Errors are typed and omit URLs,
credentials and response bodies. No discovery output is written to stdout/stderr
implicitly. Existing CLI JSON contracts are unchanged. The generated skill states
that this capability is library-only and does not teach future routing flags.

## Deterministic validation

`cargo test --lib proxy::hproxy` covers JSON/null/malformed/mixed rows, optional
fields, filtering and deduplication; shared successes and failures; declared and
chunked oversize bodies and exact EOF; full-list and explicit pagination counters;
aggregate bytes, retained entries and page limits; redirects; 429/5xx, Retry-After
seconds/dates, deadline and attempt exhaustion; cancellation and throttling with
Tokio's controlled clock; explicit bootstrap proxy injection; HTTP forwarding and
verified HTTPS CONNECT in both transports. Ordinary tests require no live service
or credentials. No performance improvement is asserted.
