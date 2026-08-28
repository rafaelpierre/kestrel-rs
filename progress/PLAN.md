# Kestrel optimization plan

The optimization rule is to preserve default behavior unless a change has
separate semantic-quality evidence. Every milestone gets unit/integration tests
and an ablation benchmark before its default is reconsidered.

## Milestone 1 — fine-grained diagnostics

Status: completed 2026-08-28

- [x] Record provider/query latency, result count, success/failure, and retries.
- [x] Record each page's queue, request/TTFB, download, parse, and total time, plus
  response size and outcome.
- [x] Record cache hits/misses and the number of fetches cancelled by a total budget.
- [x] Include aggregate diagnostics in benchmark artifacts without OpenTelemetry.
- [x] Keep normal result JSON stable and make diagnostics best-effort.

The HTTP client exposes connect-through-header time as one request/TTFB phase;
DNS, TCP, and TLS are not separately observable without lower-level client
instrumentation.

## Milestone 2 — snippet pre-ranking

Status: completed and parked 2026-08-28; retained as opt-in

- [x] Score provider title/snippet text before network fetching.
- [x] Select a bounded candidate set while retaining provider and query diversity.
- [x] Fetch the selected candidates, then run the existing content BM25 final rank.
- [x] Compare URL overlap, returned characters, requests, and latency against
  the current first-N candidate policy.
- [x] Compare semantic task score before reconsidering the default.

## Milestone 3 — adaptive fetch scheduling

Status: pending

Provider-search straggler handling landed ahead of this milestone: fanout mode
can opt into a per-query success quorum and cancel unfinished provider searches.
This is separate from adaptive page-fetch scheduling below.

Provider quorum is also parked as opt-in after the Codex comparison. Do not
revisit its default, pre-ranking defaults, or BM25 thresholds without a new
isolated quality experiment.

- Launch candidates in pre-rank order with bounded concurrency.
- Reassess after each completed extraction and stop launching work when enough
  useful documents exist or the total budget is nearly exhausted.
- Preserve every useful completion and distinguish work never launched from
  work cancelled at the deadline.

## Milestone 4 — per-host concurrency

Status: pending

- Apply a global network limit and a configurable per-origin limit.
- Ensure one throttled domain cannot occupy the full fetch pool.
- Benchmark mixed-host and same-host workloads with deterministic local tests.

## Milestone 5 — extracted-content deduplication

Status: pending

- Retain canonical-URL deduplication before fetching.
- Fingerprint normalized extracted text (initial candidate: SimHash) after
  fetching and remove near-duplicate mirrors/syndicated pages.
- Keep the highest-ranked representative and merge provenance where possible.
- Evaluate false-positive behavior on short pages and boilerplate-heavy pages.

## Milestone 6 — batch/streaming operation

Status: pending

- Add a JSONL stdin/stdout mode that reuses one `KestrelClient` and an in-memory
  cache across queries.
- Leave the one-shot CLI unchanged.
- Benchmark one-shot process latency and long-running throughput separately.

## Milestone 7 — cache and parser profiling follow-ups

Status: pending

- Add a small in-memory extracted-text cache for long-running operation.
- Avoid full cache-directory scans on frequent writes if profiling shows them
  to be material.
- Optimize DOM traversal/string allocation or evaluate streaming extraction
  only if phase timings show parsing is significant.
- Defer BM25 micro-optimization unless ranking becomes a measured bottleneck.
- Record snippet pre-rank scores in diagnostics before experimenting with a
  relevance gate. Prefer a relative-to-best threshold with a minimum-result
  floor and diversity constraints over a fixed absolute BM25 cutoff.

## Benchmark matrix

For each behavior-changing optimization, report:

- cold latency and warm latency separately;
- p50/p95 end-to-end, search, fetch, parse, and rank time;
- provider success/result counts and fetch outcome counts;
- number of HTTP requests and bytes downloaded;
- final URL overlap, content characters, and token estimate;
- semantic task score;
- Python/Rust parity only with Rust-only options disabled.
