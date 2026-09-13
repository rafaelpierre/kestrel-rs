# Current-contract streaming validation (#46)

PR #68 shipped unique-result stopping and incremental parsing. This investigation
validates that policy; it does not introduce another public stopping control.
The historical portable pilots are not comparable to the current passthrough experiment.

## Experiment

The ignored `live_streaming_validation` library test uses the production request
validator, provider jobs, incremental parser, canonical URL key, collector and
round-robin merger. Only the stopping decision is overridden in test builds:

| Arm | Parsing | Stop condition |
| --- | --- | --- |
| `full` | Incremental | All enabled providers finish, fail, or reach the common deadline |
| `batch` | Completed bodies | At least five valid unique candidates |
| `stream` | Incremental | Current production target of five valid unique candidates |
| `diversity` | Incremental | Five valid unique candidates AND two nonempty provider buckets |

A provider may corroborate an existing URL to count toward diversity. Diversity
is measured across retained candidates, not guaranteed in the final five. Every
arm retains chunk/batch overshoot and uses provider passthrough, with URL/site checks.
Metadata schema version 2 labels `query_syntax` as `passthrough`; portable mode
and its parser have been removed. Earlier schema-1 portable artifacts remain
historical and must not be pooled with this experiment. All arms use the same retry
behavior, deadline, nine providers and concurrency nine. Full fanout is bounded
by that deadline; it cannot establish exhaustive provider coverage. It continues
to emit snapshots so time-to-five remains observable. First time-to-five can
precede a subsequent error that retracts provisional records; returned target
coverage is reported separately.

The versioned eight-query corpus covers general, technical/coding, long-tail,
version-specific, phrase and site-filter retrieval. It is a starting corpus, not
proof of representativeness for every workload. Each query includes a judgment
intent. All four arms rotate order by query and trial and run with fresh and
reused clients. Reused pools are separate per arm and initially cold; there is
no excluded warm-up. One fixed browser/OS/language profile is used throughout.
There is 250 ms pacing after each call. Upstream caches and network state are
uncontrolled. Search timings exclude client construction, page fetching and body
ranking; the final five use production round-robin fusion.

## Reproduction

Run from the issue worktree. Choose a new output directory for each independent
network/time window; the runner refuses to reuse a directory. The context value
should describe region/network conditions without private addresses or secrets.

```sh
mkdir -p benchmarks/results
KESTREL_VALIDATION_OUTPUT=benchmarks/results/streaming-validation-window-1 \
KESTREL_VALIDATION_CONTEXT='describe region and network type' \
cargo test --release --lib live_streaming_validation -- --ignored --nocapture

python3 benchmarks/streaming_validation_report.py \
  --input benchmarks/results/streaming-validation-window-1 \
  --output benchmarks/results/streaming-validation-window-1/summary.json
```

Defaults: three trials, eight queries, two client conditions, four policies =
192 searches, with a three-second search budget. `KESTREL_VALIDATION_TRIALS`,
`KESTREL_VALIDATION_QUERIES` (a corpus prefix, 1–8) and
`KESTREL_VALIDATION_BUDGET` are positive integer overrides for a smaller pilot.
A prefix is a smoke test and must not be presented as the full corpus. Keep
budgets matched across arms. Use release builds for performance observations.

`metadata.json` records corpus, configuration, environment description, compiler,
revision, SHA-256 of the exact tracked diff stdout bytes (without text decoding
or trimming), and test executable hash. Stage added source files
before running so the tracked diff hash includes them. `runs.jsonl` checkpoints
one complete record per finished call; `COMPLETE` is written only after all
scheduled calls. A killed process may leave a truncated last line: preserve the
original artifact and explicitly document any recovery into a new artifact.
The summary rejects duplicate/unexpected run identifiers, detects missing runs,
includes input hashes and refuses to overwrite its output. Generated artifacts
remain ignored and local.

## Reading the evidence

The summary separates fresh/reused arms, errors, target coverage, all-run and
successful-search latency, paired latency differences, collector time-to-five,
decoded bytes, provider contributions and canonical top-five slot overlap.
Percentiles use the nearest-rank definition; small-sample p95 is descriptive.
Failure/empty searches remain in all-run summaries. No winning samples are
selected or discarded. An empty pair has zero slot overlap, not perfect overlap.

The nine-provider matrix reports observed protocols/statuses, outcomes,
headers/first-record/fifth-record/EOF timings and pre-EOF records. Missing provider
observations are counted explicitly. Existing probe timings aggregate a logical
search, including retries; they are not a per-attempt feasibility trace.
Cancelled or blocked responses do not establish whether an adapter supports
incremental delivery. Provider events and retained provenance are both preserved
in each run. Total-failure runs retain diagnostics rather than losing them when
the production merger returns an error.

`return_after_threshold_ms` measures local elapsed time between observing a stop
condition and finishing search/fusion. It is not a server acknowledgement or
proof that upstream work stopped. Existing deterministic HTTP/1.1 and HTTP/2
transport tests cover scoped cancellation and shared-pool safety separately.
Decoded bytes are bytes accepted by the existing body probe, not compressed wire
traffic (an oversized rejected chunk is excluded). Incremental parser/worker
elapsed time includes worker scheduling and excludes final batch parsing; it
cannot be used as an isolated CPU comparison.

Optional judgments are a versioned JSON file containing:

```json
{"version": 1, "judgments": {"general": {"https://example.org/": true}}}
```

Use the exact returned URL, with `true` for relevant, `false` for irrelevant,
and `null` or no entry for unknown. Pass `--judgments PATH` to the report.
Precision@5 is computed only for error-free runs whose returned URLs are all
judged; missing slots then count as zero. An unjudged returned URL remains
unknown. Preserve judgment versions and add separately documented evidence,
source authority, freshness/version correctness and direct-result-loss review
before making a policy recommendation. URL overlap alone is not relevance.

## Remaining investigation

The harness and a single pilot do not close #46. Still required are repeated
independent time windows, a reviewed representative corpus, relevance/evidence
judgments, independent provider probes when fanout censors observations, and
per-attempt format/compression/buffering evidence. Isolated parser CPU, peak RSS,
compressed wire traffic and server-side cancellation latency remain unmeasured
and are explicitly listed in metadata. Coordinate independent provider evidence
with #29 rather than duplicating that broader investigation. #75 owns the
existing CLI benchmark corrections and CLI ablations; #80 owns process/startup
and deadline-overhead attribution.

Recommendation at this stage: make no further default-policy change on the
strength of historical pilots or overlap alone. Keep #46 open until current
quality and resource evidence supports a decision.

## Deterministic checks

```sh
cargo test --lib search::streaming::probe::validation::tests
python3 -m unittest discover -s benchmarks -p test_streaming_validation_report.py
```

Production CLI help, library contracts and installed skill behavior do not change:
the override and instrumentation hook compile only under `cfg(test)`. No new
dependencies or Rust language/library requirements are introduced.

## Initial current-contract pilot — 2026-09-13

All 64 scheduled searches completed: the full eight-query corpus, one trial,
four policies and two client conditions. Release build on macOS arm64; fixed
Chrome 146/macOS profile; three-second deadline, concurrency nine, 250 ms pacing.
Network region and upstream routing were not independently verified. Other
worktrees were compiling on the same host during the experiment. These are
exploratory observations with host contention and live-network variance, not a
reliable speedup or tail-latency comparison. Eight observations per table row
are insufficient to establish a stable p95. No relevance judgments were made.

| Policy | Clients | Search p50 ms | Observed p95 ms | Returned 5+ candidates | Returned 2+ providers |
| --- | --- | ---: | ---: | ---: | ---: |
| full | Fresh | 3002.0 | 3003 | 3/8 | 1/8 |
| batch | Fresh | 2323.5 | 3003 | 4/8 | 1/8 |
| stream | Fresh | 1639.0 | 3002 | 5/8 | 1/8 |
| diversity | Fresh | 3002.0 | 3007 | 3/8 | 0/8 |
| full | Reused | 3002.0 | 3006 | 4/8 | 0/8 |
| batch | Reused | 2164.0 | 3002 | 4/8 | 1/8 |
| stream | Reused | 1930.5 | 3003 | 4/8 | 0/8 |
| diversity | Reused | 3001.5 | 3002 | 5/8 | 1/8 |

The full reference sometimes returned fewer candidates than an early-stopping
arm in a different call. Sequential live requests are not identical provider
responses, so this is not evidence that truncating the same input improves
coverage. The matrix below includes every scheduled provider (64 logical
searches per provider); failure and cancellation remain observations, not proof
of parser infeasibility.

| Provider | Observed protocols | Logical outcomes |
| --- | --- | --- |
| duckduckgo | Not observed | cancelled_min_results: 18, deadline: 46 |
| bing | HTTP/2.0 | filtered_empty: 56, results: 8 |
| yahoo | HTTP/2.0 | cancelled_min_results: 5, request_error: 59 |
| dogpile | HTTP/2.0 | request_error: 64 |
| ecosia | HTTP/2.0 | request_error: 64 |
| swisscows | HTTP/2.0 | request_error: 64 |
| yep | HTTP/2.0 | cancelled_min_results: 10, deadline: 3, empty: 14, filtered_empty: 15, results: 22 |
| qwant | HTTP/2.0 | request_error: 64 |
| mojeek | HTTP/1.1 | challenge: 10, request_error: 54 |

Only Bing and Yep contributed retained provenance. No policy change is
justified by this pilot: provider availability, host load, missing quality
judgments and missing resource instrumentation prevent that conclusion.
Independent provider measurements and repeated quieter windows remain necessary.

Local artifacts: `benchmarks/results/streaming-validation-pilot-20260913/`
(`metadata.json`, `runs.jsonl`, `COMPLETE`, `summary-v2.json`). They are ignored
and are not included in the PR; the table is a bounded report of this run.
Use `KESTREL_VALIDATION_TRIALS=1` with the reproduction command for the same
scheduled design in a new directory; network outcomes will vary.

Raw runs SHA-256: `c0e01fe7ecccd3219e90bc0d15a5189413068802ca32604d1b5d157e01b3f068`.
Test binary SHA-256: `219b2c4f950813ba13e02b976abdabc64d7404880ef3f1af40696daf7d818d6a`.
Source base: `a8fe1e11c3f91fa4d89183c6f75e83b5aba2f5ab`; legacy trimmed-text diff digest at measurement:
`45a8a6f337415a793b83cbbb1588e0ea0f7cf7c5437cb8db7fc934c10db35216`. The report fields and this documentation were
completed after the run. Review subsequently fixed provenance capture to hash
raw diff bytes, so this historical trimmed-text digest must not be compared with
a raw diff SHA-256. The pilot artifacts remain unchanged; new runs use the fixed
raw-byte capture. The review fix also makes the report help show the complete
versioned judgments envelope.

Pilot validation passed: formatting, Clippy with warnings denied, 178 Rust tests
(five ignored), four new collector/policy regression tests included in that
count, three Python reporting tests, and the release live pilot. An initial
full-suite run failed two existing 200 ms diagnostics phase assertions during
concurrent compilation; the unchanged suite passed on rerun. Follow-up #124
records that test-reliability observation. The production release executable
was not separately rebuilt: all Rust behavior changes are test-only, and the
live pilot built and ran the optimized library test executable.

Review-fix validation: formatting, Clippy, 179 Rust tests (five ignored), and
four Python report tests passed. Added coverage preserves raw diff whitespace
and non-UTF-8 bytes, and runs the report using the JSON envelope extracted from
its own help text. The live pilot was not rerun for these provenance/help fixes.

## Independent provider probes

`live_streaming_provider_probes` runs all eight corpus queries against each of
nine providers separately (72 sequential calls), rotating provider order by
query. Each call uses a fresh client with the same fixed browser profile as the
four-policy experiment. The test-only full policy disables target stopping;
only provider completion/failure or the common deadline ends the call. This
removes competing-provider cancellation, while deadline and retry censoring
remain. It preserves diagnostics on total failure and checkpoints each result.

```sh
KESTREL_VALIDATION_OUTPUT=benchmarks/results/streaming-independent-window-1 \
KESTREL_VALIDATION_CONTEXT='describe region and network type' \
KESTREL_VALIDATION_BUDGET=3 \
cargo test --release --lib live_streaming_provider_probes -- --ignored --nocapture
```

Choose a new directory; existing directories are rejected. The probe writes
`metadata.json`, `runs.jsonl` and `COMPLETE`; its separate experiment schema is
not input to the four-policy summarizer. Retain all 72 records, including empty
results, HTTP failures and deadlines. Content type is now captured by the shared
test probe alongside protocol/status. Missing content type means unobserved;
returned content type describes the server's declaration, not a verified layout.
The transport exposes decoded bodies, so absent Content-Encoding cannot establish
uncompressed wire delivery. First chunk/record/EOF spacing does not distinguish
server buffering from intermediary buffering. These require per-attempt wire
instrumentation coordinated with #29. No production behavior or installed-skill
contract changes.

For a position-balanced four-policy run, set `KESTREL_VALIDATION_TRIALS=4`:
each policy occupies every position once per query/client condition, for 256
searches. Fresh clients still precede reused clients; this is a limitation of the
existing design, not a randomized estimate of pooling benefit. Separate complete
time windows must remain distinct, and same-host repetitions do not establish
geographic or independent-network representativeness.

## Repeated current-contract measurements

See [the 512-call comparison and 72 independent probes](streaming-evidence-2026-09-13.md)
for measured quality, resource and provider limitations, the recommendation, and
the canonical gate (NOT PASSED, 5/10). This supersedes the historical pilot as
current observations, while retaining its artifacts. Issue #46 remains open.
