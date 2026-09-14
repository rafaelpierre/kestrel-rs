# Provider representation reuse (#116)

Completed responses are classified and extracted within one admitted provider
worker. Transport owns status handling, retries and diagnostics publication;
adapters retain extraction and rejection rules. A `Send` result crosses the worker
boundary, never a scraper DOM. This removes the second worker admission and second
root parse for HTML responses and Qwant JSON. Other JSON adapters retain one root
decode. No dependency, public API, CLI option or output schema changes.

## Adapter inventory

| Adapter | Completed response | Incremental / fallback inputs |
| --- | --- | --- |
| DuckDuckGo | One HTML DOM for diagnostics and native challenge/empty/result extraction | Persistent tokenizer emits closed cards; complete EOF body independently validates the page |
| Bing | One HTML DOM for diagnostics, challenge/empty validation and results | Closed cards; EOF full-page validation |
| Yahoo | One HTML DOM, including the separate impersonating transport | Closed cards; EOF full-page validation |
| Ecosia | One HTML DOM; firewall diagnostics and adapter rejection retain their separate rules | Closed cards; unknown layouts use EOF fallback |
| Mojeek | One HTML DOM; existing title/message CAPTCHA and empty-page rules | Closed cards; EOF fallback validates page-wide markers |
| Dogpile | One JSON root; result markup is data | Array-path probes and successive closed-item snapshots; EOF validates the complete root |
| Yep | One JSON root, retaining the `Ok` envelope check | Probes/snapshots preserve the `/1/results` path; EOF full-root validation |
| Qwant | One JSON root shared by top-level challenge URL detection and extraction | Each snapshot now shares one value for classification/extraction; probes and complete EOF remain separate inputs |
| Swisscows | One outer JSON root, plus one decoded payload root when an encoded envelope is present | Direct `items` can stream; encoded payload uses completed fallback |

JSON title/snippet HTML fragments still require fragment extraction. An encoded
Swisscows payload is a different wire representation, not a repeated parse of the
outer JSON. Array probes contain a sentinel and synthetic closing delimiters;
closed-item snapshots contain evolving partial response data. The complete EOF
body must validate trailing syntax and page-wide fields even when records streamed.
The existing pass/depth bounds, framing and result-minimum stopping stay unchanged.
This is one parse per completed representation and per snapshot, not a claim of
one parse for an entire streaming request including every evolving prefix.

Diagnostic challenge detection is advisory. In particular, broad diagnostic
markers must not silently become new native DuckDuckGo/Ecosia rejection rules.
The existing streaming dispatcher retains its own validation rules. HTTP failures
skip extraction and preserve retry/status behavior; a successful HTTP response
with malformed provider content returns the extraction error without retrying it.

## Deterministic validation and profiling

`cargo test --lib completed_` covers all nine fixtures, malformed/empty/challenge
boundaries, encoded and malformed Swisscows envelopes, one root representation per
completed processing call, and both HTTP transports without parse-error retries.
Existing streaming tests compare all nine adapters at multiple chunk sizes and
cover cancellation, deadlines, worker capacity and persistence.

For an external allocation profiler, build the library test executable using
`cargo test --lib --no-run`, then run its ignored
`search::parse_once_tests::completed_transport_parse_profile` test with
`--ignored --exact ... --nocapture`. Set `KESTREL_PARSE_PROFILE_ENGINE` to `bing`
or `qwant` and `KESTREL_PARSE_PROFILE_MODE` to `duplicate` or `shared`. The duplicate
arm reproduces separate transport classification and completed extraction; the
shared arm uses one representation. Both assert 1,000 results over ten iterations.
Use fresh processes, rotate arm order, and record executable hash, build profile,
OS, profiler and raw outputs. Disable remote telemetry for these local fixtures.

Fixture creation and one extraction warm-up are excluded from logged timing but
included in whole-process allocation profiles, as are test-harness allocations.
Report allocation counts/bytes separately from retained memory and timing; profiler
overhead makes its timings unsuitable for ordinary latency claims. Record
unprofiled paired timings separately with nearest-rank p50 and sample size.
Network, queueing and live provider quality are outside this local measurement.
Keep raw profiles local and publish measured results with the PR. This complements
the required Rust checks, generated-skill installation and live ten-question gate.
