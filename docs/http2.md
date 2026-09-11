# HTTP/2 transport tuning

Kestrel negotiates HTTP/2 over TLS with HTTP/1.1 fallback. All entry points use the
shared transport policy. For repeated searches/fetches, retain a `KestrelClient`:
cloning it shares connections, DNS caches and the underlying TLS configuration.
Convenience functions create pools for each call; a new CLI process cannot reuse
connections from a previous process.

```rust,no_run
use kestrelsearch::{KestrelClient, TransportOptions, Engine};
use std::time::Duration;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let options = TransportOptions {
    // Only enable idle PING if your intermediary requires it.
    http2_ping_interval: Some(Duration::from_secs(60)),
    http2_ping_while_idle: true,
    ..Default::default()
};
let client = KestrelClient::with_transport(options)?;
let warmup = client.warm_up_search(&[Engine::Bing], Duration::from_secs(3)).await?;
// Fetch origins use a separate pool; warm them when you know the targets.
let pages = vec!["https://example.com/article".to_owned()];
let warmup = client.warm_up_fetch(&pages, Duration::from_secs(3)).await?;
let shared = client.clone();
# Ok(())
# }
```

Warm-up is opt-in and performs HEAD requests. Fetch URLs are deduplicated by
scheme/host/port and reduced to `/`; credentials, paths and queries are discarded.
Search warm-up uses the actual provider origin (including Yahoo's impersonated
pool). Up to four requests run concurrently, each with its own timeout. Results
preserve origin order and include status, protocol, elapsed time and any error.
Normal redirects apply, so a redirect may warm a different final origin. A HEAD
405 still establishes a transport; an origin may close it afterward. Warm-up is
not a guarantee that a subsequent request avoids setup. Await warm-up before the
latency-sensitive phase, rather than adding it immediately before every request.

## Defaults and interpretation

| Setting | Default | Purpose |
| --- | --- | --- |
| Idle timeout | 300 seconds | Keep healthy connections warm without permanent idle retention |
| Maximum idle per host | 2 | Bound idle pooling; **not** a cap on active sockets or streams |
| Connect timeout | 10 seconds | Bound cold connection setup; request deadlines still apply |
| Stream receive window | 1 MiB | Allow moderate responses to progress without tiny windows |
| Connection receive window | 4 MiB | Support concurrent streams with bounded initial credit |
| Adaptive windows | Off | Predictable fixed credit; opt in after bandwidth/RTT measurements |
| HTTP/2 PING | Off | Avoid unsolicited traffic and server enforcement problems |
| PING timeout | 10 seconds | Detect unanswered PINGs when enabled |
| TCP keepalive | 60 seconds | Detect dead peers; independent of HTTP pool retention |
| TCP_NODELAY | On | Avoid Nagle delays on small requests |
| DNS cache | 256 names / 60 seconds | Reuse successful system-resolver answers across reconnects |

Options validate before client construction. Fixed stream windows allow 65,535
bytes through 16 MiB; connection windows allow 65,535 bytes through 64 MiB. These
are flow-control credits, not hard total memory bounds. Adaptive mode overrides
fixed windows and may grow them. Existing response-size and concurrency limits
remain in force. PING intervals must be at least 30 seconds; the server may need
a longer interval. Idle PING requires an explicit interval.

DNS uses the operating system resolver, preserving hosts-file and VPN behavior.
The lifetime is an application freshness bound, **not an authoritative DNS TTL**.
Expired entries are removed on cache access, the oldest entry is evicted at
capacity, and empty/failed lookups are not cached. Concurrent cold misses may
perform duplicate lookups; cache locks are never held during network I/O. DNS
expiration affects the next connection and does not recycle healthy sockets.

Native TLS defaults negotiate TLS 1.3 when supported, retain compatible fallback,
and manage session resumption within the retained client configuration. Kestrel
does not enable 0-RTT, persist tickets across processes, force TLS 1.3 on old
origins, or rebuild TLS configuration on each request. Resumption depends on the
server and TLS backend. Search and page-download pools stay separate so bulk
pages do not share the search connection. Browser impersonation is retained for
Yahoo, with flow-control settings overridden by the transport policy.

## Diagnostics

`fetch_all_detailed` records the negotiated `http_version` once response headers
arrive, including rejected responses. It is absent for cache hits and failures
before headers. Older JSON without this field remains readable.

| Timing | Meaning |
| --- | --- |
| `queue_ms` | Application network semaphore wait |
| `request_ms` | `send()` until response headers; includes internal queueing, DNS/TCP/TLS if cold, redirects and server response |
| `download_ms` | Reading the response body |
| `parse_queue_ms`, `parse_ms` | Parser admission and extraction |
| `total_ms` | Complete page operation |

The libraries do not expose separate pool wait, stream-slot wait, DNS, TCP,
TLS or request-send timestamps at this layer. `request_ms` is neither pure server
latency nor exact first-byte timing. Existing failed-request diagnostics retain
total time but may have zero phase timings; zero is not evidence of zero wait.
Provider reports retain aggregate provider durations.

## Benchmark before expanding the pool

Run deterministic local checks and a repeatable cold/warm transport benchmark:

```sh
cargo test --all-features
cargo test --lib transport::tests::benchmark_cold_and_warm_h2 -- --ignored --nocapture
```

The benchmark samples 100 pairs of requests: a fresh client connection followed
by a request on that same connection. It reads 1 KiB responses, prints p50/p95/p99
in microseconds and verifies 100 connections served 200 requests. Client
construction is outside the timed region. It uses cleartext H2 on loopback to
isolate connection setup/reuse; it does not measure TLS, DNS or WAN latency.
Unit tests also exercise simultaneous streams, server stream limits, transfers
larger than windows, impersonated H2, warm-up and HTTP/1.1. A local TLS test uses a fresh test
certificate with verification enabled to check TLS 1.3, ALPN H2 negotiation,
HTTP/1.1 fallback and socket reuse for both transport backends.

For production tuning, use a controlled HTTPS endpoint and compare full-response
latencies across cold/warm phases, realistic concurrency, small and large bodies,
and representative RTT/loss. Keep payloads and compression constant, collect
p50/p95/p99 plus throughput/error rates, and inspect protocol and queue timings.
Do not infer an Internet speedup from loopback numbers.

The current libraries multiplex on a retained H2 connection and honor the peer's
`SETTINGS_MAX_CONCURRENT_STREAMS`. `max_idle_per_host = 2` does not create two H2
connections. Only consider explicit 2–4 connection sharding after measuring
stream saturation, flow-control stalls or packet-loss head-of-line blocking.
Cross-origin coalescing is not promised; avoid unnecessary origin sharding.
There is no periodic forced lifetime or fleet reconnect schedule requiring
jitter. If service discovery needs active recycling, design that policy with its
actual freshness constraints. Header size, body compression CPU tradeoffs and
Linux BBR/fq require workload/platform benchmarks; they are not changed blindly.

## Recorded local measurement

On 2026-09-11, the debug-build loopback benchmark on the development Mac
returned the following values (100 requests in each group):

| Phase | p50 | p95 | p99 |
| --- | --- | --- | --- |
| Cold connection | 1.743 ms | 2.138 ms | 2.392 ms |
| Warm connection | 1.421 ms | 1.568 ms | 1.982 ms |

These numbers describe this run only. The fixture includes an asynchronous
server scheduling step; timings include that scheduling overhead. This compares
cold and warm connections using the new policy, not before/after policy settings.
TLS resumption and DNS-cache latency improvements are not measured here.
