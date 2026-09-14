# Search module ownership

`search.rs` is the public search façade. It validates requests, owns retained
clients and diagnostic lifetimes, and orchestrates provider fanout. Public
functions, provider order, options, errors and result schemas are unchanged.

| Internal module | Responsibility |
| --- | --- |
| `providers.rs` | Additional-provider wire contracts and dispatch for Dogpile, Ecosia, Swisscows, Yep, Qwant and Mojeek; public browser-facing `search_url` |
| `providers/{bing,duckduckgo,yahoo}.rs` | Native request construction and provider-specific completed HTML extraction |
| `providers/response.rs` | Worker-local HTML/JSON representation, challenge classification and completed extraction dispatch |
| `providers/html.rs` | Thread-local constant selector cache and shared HTML text helpers |
| `providers/records.rs` | Bounded incremental HTML/JSON framing and extraction using the same adapter functions |
| `search/transport.rs` | Backend HTTP execution, existing retries, bounded decoded-body reads, charset decoding and transport diagnostics |
| `search/parsing.rs` | Shared parser admission and worker-owned permits that survive caller cancellation |
| `search/results.rs` | URL eligibility, original ranks, provenance, canonical deduplication and deterministic round-robin fusion |
| `search/streaming.rs` | Provider-independent publication, acknowledgement, partial-result retention and collection stopping |
| `search/discovery.rs` | Bounded discovery recovery across incomplete providers |
| `error.rs` | Library-wide error definition independent of orchestration |

Adapter and transport entry points are crate-internal. They use explicit imports;
no external adapter/plugin interface is introduced. Providers construct requests
and supply extraction callbacks to transport. Transport reads bounded bodies and
submits completed classification/extraction to the parser pool. DOMs stay within
the worker; only owned results cross async boundaries. Streaming record parsers
use the same pool and completed adapter dispatch, then publish owned snapshots to
the collector. Normalization runs before provenance and fusion for both paths.

The public error type remains available through both `kestrelsearch::KestrelError`
and `kestrelsearch::search::KestrelError`. Variants, conversions and display text
are unchanged. `search::MAX_PROVIDER_RESPONSE_BYTES` remains the public limit.
The error module itself is private; moving it does not add a public module API.

This extraction preserves the parse-once and worker-ownership work from #116/#117.
It preserves the shared backend retry policy from [#205](https://github.com/rafaelpierre/kestrel-rs/issues/205)
in `search/transport.rs`, including backend-specific send-error eligibility.
The typed provider outcomes from [#206](https://github.com/rafaelpierre/kestrel-rs/issues/206)
are preserved across adapter and transport boundaries. `error.rs` owns the public
error; `search/failure.rs` retains internal failure identity until the public
boundary. Diagnostic lifetime and recovery decisions remain in search.
No new categories or serialized values are introduced by this extraction.

Validation retains the nine-adapter completed/streamed fixture suites, request
encoding and Bing fidelity tests, parser bounds, cancellation/deadline tests,
transport diagnostics and response-size tests. A cross-boundary fixture checks
that rejected URLs retain original ranks, canonical duplicates retain ordered
source occurrences, and one failed provider does not discard successful results.
An external integration test compiles both public error paths and checks display
and conversion compatibility. CLI and generated-skill contracts are unchanged;
the final binary's skill is still installed in a temporary project for validation.
