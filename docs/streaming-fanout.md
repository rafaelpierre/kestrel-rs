# Streaming fanout

Fanout starts enabled providers concurrently, subject to `max_concurrency`. Its
per-query minimum is **five valid, unique results** unless `--min-results N` /
`SearchOptions::min_results` overrides it. There is no separate streaming flag.
The collector cancels unfinished requests immediately after the minimum is met.
It does not wait for every provider. The result minimum overrides
`provider_quorum`, which remains accepted for compatibility. Fusion combines
available sources and does not require a minimum number of providers.

## One result contract

All adapters deliver `SearchResult`: `title`, `url`, `display_url`, `snippet`,
`engine`, `query`, `engine_rank`, and `sources`. The same normalization function
assigns original ranks and applies URL validation and query constraints for streamed and completed
responses. URL canonicalization is shared with fusion, so duplicates cannot
inflate the minimum. A provider snapshot replaces its previous snapshot; updates
do not double-count its records or provenance.

Only complete records from successful HTTP responses are published. Errors,
known challenges, empty responses and filtered-out records contribute zero.
A failed/malformed response before the threshold retracts that provider's snapshot.
A deadline retains already completed records. Exhausted providers or the deadline
can therefore return fewer than five results. The target is a stopping threshold,
not an output cap: one received chunk can contain additional complete records.

Each query has its own collector. The bounded channel and acknowledgement make
providers pause before reading their next chunk until the collector checks the
target. On stopping, it drops pending provider futures, including requests queued
on the concurrency semaphore and retries/backoff, and ignores unread tails.
HTTP/2 connection pools remain shared; cancellation is scoped to the request's
streams. HTTP/1.1 can need a replacement connection after an unfinished body is
cancelled. A cancelled upstream server may already have performed work or sent
bytes; local cancellation cannot undo that work.

## Provider extraction

| Provider | Incremental format | Fallback |
| --- | --- | --- |
| DuckDuckGo | Closed HTML web-result cards | Unrecognized layout parsed at EOF |
| Bing | Closed `li.b_algo` cards | Unrecognized layout parsed at EOF |
| Yahoo | Closed `div.dd.algo` cards | Title-only alternative layout parsed at EOF |
| Ecosia | Closed result/web-result cards | Unrecognized layout parsed at EOF |
| Mojeek | Closed list items within `results-standard` | Unrecognized layout parsed at EOF |
| Dogpile | Complete objects in `results` | Unsupported envelope parsed at EOF |
| Yep | Complete `results` objects after the `Ok` status | Unsupported envelope parsed at EOF |
| Qwant | Complete web-row `items` after successful status/type metadata | Later metadata ordering parsed at EOF |
| Swisscows | Complete plain `items` objects | Encoded `payload` envelope parsed at EOF |

A provider falling back to EOF still feeds the same collector and is cancelled
when other providers satisfy the target. Transport chunks need not align with
UTF-8 characters, tags, strings or JSON records. Both clients decompress before
incremental character decoding. HTML uses a persistent tokenizer on a blocking
worker, parsing each completed card once. A bounded worker channel ends when its
provider is dropped. JSON recognizes the exact result-array location, including
nested envelopes, and never treats a completed nested field as a completed result.

The existing 4 MiB decompressed-body limit applies. HTML/JSON nesting is bounded
at 128 levels. JSON prefix decoding has a 256-pass/probe limit and falls back to
EOF after that limit. No extra response suffix validation delays early stopping.

## Verification

The normal test suite covers all nine adapters against their existing full-response
parsers; split records and UTF-8; nested JSON; scripts/comments; challenge and
HTTP error rejection; URL filtering; duplicate counting; original ranks and
provenance; simultaneous queries; deadlines; and cancellation diagnostics.

Local HTTP servers test reqwest and primp with HTTP/1.1 and HTTP/2, including gzip
bodies that never finish. The collector returns complete results and releases
pending reads. Other responses remain usable and HTTP/2 pools are reused.

## Historical live pilot, 2026-09-12

These measurements predate portable query filtering (#54) and do not measure the
current implementation. The pilot compares **two early-stopping modes**, both with a minimum of
five, no provider quorum, concurrency nine and a five-second deadline. The batch
reference is test-only and disables incremental parsing. Neither mode waits for
all providers. Three queries run with fresh and reused clients, with alternating
mode order: six observations per mode, twelve searches total. Page fetching and
ranking are excluded to isolate search. These are debug-build, single-environment
measurements, not a statistically reliable performance guarantee.

| Measurement | Completed-batch early stop | Streaming early stop |
| --- | ---: | ---: |
| Median search latency | 206.5 ms | 155 ms |
| Observed nearest-rank p95 (six samples) | 287 ms | 262 ms |
| Median decompressed bytes received across providers | 50,091.5 | 33,875 |

The median streaming parser/worker-wait time was 831.5 microseconds. This includes
worker scheduling and is not isolated CPU time. Peak process RSS and compressed
wire-byte counts were not measured. An earlier prefix-reparsing prototype was
slower; the final implementation retains tokenizer state and parses cards once.

Bing produced complete records before body EOF in the final streaming pilot.
Swisscows often supplied the minimum first using its completed encoded envelope.
Dogpile and Ecosia returned HTTP 403; Qwant returned 403 when its headers arrived.
Mojeek returned HTTP 403 or a known challenge. Other requests were cancelled
before useful results arrived. Those outcomes contributed zero to the minimum
and do not establish whether those providers generally support incremental delivery.
Observed protocols included HTTP/2 for Bing/Swisscows and HTTP/1.1 for Mojeek;
the local fixtures separately verify both transports for both clients.

The two modes had identical top-five URL sets in four of six matched pairs.
The other overlaps were 0/5 and 2/5 because the winning provider changed. The
Rust official ownership chapter appeared in both modes in both runs. In the
fresh PostgreSQL pair, streaming retained EXPLAIN documentation while the batch
reference returned general PostgreSQL pages. This small spot-check establishes
neither a broad relevance improvement nor equivalence. Early stopping deliberately
trades provider coverage for latency. Provider quorum no longer delays the result target.

Reproduce the opt-in network pilot with:

```sh
cargo test --lib live_streaming_feasibility -- --ignored --nocapture
```

It writes `benchmarks/results/streaming-feasibility.json`, including per-provider
header, first-chunk, first-record, fifth-unique-record and EOF timings; decoded
bytes; parser/worker time; cancellation outcomes; and retained results. Normal
tests never contact live providers. Provider availability and winning results
can vary between runs.

## Repeated four-query fusion evidence

See [the 48-run comparison](fusion-evidence-2026-09-12.md) for four final result
sets, provider inputs and source ranks, deduplication evidence, every matched
latency pair, and limitations. Both arms stop early at five unique records; a
historical two-provider quorum was used specifically to demonstrate fusion.
Those measurements predate portable query filtering and result-minimum precedence; that quorum no longer
delays current searches.

## Compatibility

This intentionally changes the default from completed-provider fanout to a
five-candidate minimum. `--provider-quorum` remains accepted but cannot delay the
minimum, and `--no-search-budget` only removes the deadline. Increase
`--min-results` to collect more candidates; it is not a guarantee of provider
diversity or of five fetched pages. Library callers using exhaustive
`SearchOptions` struct literals must supply `min_results` (or use
`..Default::default()`), and exhaustive `KestrelError` matches must handle
`SearchDeadline`.
