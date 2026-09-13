# Streaming policy evidence — 2026-09-13 (#46)

The current five-candidate streaming policy returned quickly but was not supported
as quality-equivalent to deadline-bounded full fanout in this sample. Across 128
calls per policy, a selected, subsequently verified supporting URL appeared in
76 full-fanout top fives, 42 batch top fives, 43 streaming top fives and 72 diversity
top fives. These are **lower bounds from selected page reads**, not fully judged
answer-correctness rates. All 512 policy calls and 72 independent provider calls
are retained; no winning subset was selected.

Recommendation: retain the incremental parser, and revisit the collection-policy
decision with provider-fidelity controls. Do not change production defaults on
this study alone. Broadly unrelated Bing results, provider blocks/rate limits,
within-window availability changes and missing judgments prevent a general policy
recommendation. Two-provider diversity often waits until the deadline and is not
an adequate standalone quality test. #32 owns Bing fidelity, #29/#119 own the
broader provider matrix, and #120 owns scheduling experiments. Keep #46 open for
representative independent windows and the remaining measurement gaps below.

**Canonical gate NOT PASSED (5/10). This work is a draft, not merge-ready.**
The five failures are retained and linked to [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148).

## Design and provenance

The [versioned eight-query corpus](../benchmarks/streaming-queries-v1.json) covers
general, science, Rust, PostgreSQL, long-tail Tokio, exact-version Rust release,
quoted-phrase and official-site retrieval. This is a reviewed exploratory spread,
not a representative workload sample or held-out benchmark. Both windows use four
rounds × eight queries × fresh/reused clients × four policies = 256 calls each.
Each policy occupies every position once per query/client condition. Fresh clients
always precede reused clients; pooling comparisons retain that order confounder.
Separate per-policy reused pools start cold, without an excluded warmup.

All arms use current passthrough semantics, the same production URL/site checks,
canonical deduplication, provider round-robin fusion, first five output selection,
minimum five, nine providers, concurrency nine, three-second deadline, existing
retries, fixed Chrome/macOS profile and 250 ms between calls. No page fetching,
body ranking or page cache runs inside measured calls. Production defaults are
unchanged. Full fanout disables the result target; batch waits for complete bodies;
stream is the shipped incremental target; diversity additionally requires two
nonempty provider buckets (including corroborating duplicate URLs). The latter
requirement is across retained candidates, not necessarily the final five.

macOS arm64, shared desktop; egress region unverified. Other workloads were
uncontrolled. Initial Rust checks overlapped early window-1 observations; the
unchanged suite rerun passed. The two windows are consecutive same-host time
windows separated by independent probes, **not independent days/networks**.
No agent-latency or causal speedup claim follows from these observations.

- Source base: `e7527719feff0265ea34ee7334f289f076162b76`; `rustc 1.96.0 (ac68faa20 2026-05-25) (Homebrew)`.
- Exact raw tracked diff SHA-256: `738f4baab72fb31d1e8706ce5c8ee946b4ea3540ff5efef98cc1cc39cc886dd7`; retained as `tested.diff`.
- Optimized test binary SHA-256: `659a06ac2aebf409eeb77e2cdfd9e029f79c6997d8f13d538d6565b7f0bbfbc4`.
- CLI: `kestrel 6.0.0`; SHA-256 `86a9d784ead41ced9af0db2ec3f4312874b68d0a8eb37884ff56a8dfb93a1e69`.
- CLI path: `/Users/rafaelpierre/projects/kestrel-rs-issue-46/target/release/kestrel`.
- Streaming corpus SHA-256: `7c59e9dea5e7a2a7db28fc3726647c2df70c0ec0fce1f3fed4b02ee95f0dcdc3`.
- Canonical dataset SHA-256: `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
- Generated skill SHA-256: `4934c1c68e4c4fd56c27f7b205f436f6756e2dd3f4604b4db62d8ffc3290d986`.
- window-1: 2026-09-13T17:00:57.806357+00:00 → 2026-09-13T17:08:17.149862+00:00; process wall 439.173 s; exit 0.
- independent: 2026-09-13T17:08:17.150266+00:00 → 2026-09-13T17:09:25.539227+00:00; process wall 68.389 s; exit 0.
- window-2: 2026-09-13T17:09:25.540444+00:00 → 2026-09-13T17:16:48.291808+00:00; process wall 442.750 s; exit 0.

The tested identity is the base plus retained raw diff, not the base alone. This
report and its methodology link were added afterward. Executable inputs, corpus,
generated skill and evaluation policy were verified unchanged before publication;
only documentation changed after the recorded runs.

## Search latency, bytes and overshoot

Each row contains all 32 scheduled calls. p50 is the ordinary sample median
(averaging the middle pair); p95 uses nearest rank `ceil(0.95*n)`. Small-sample
p95 is descriptive. Search clocks exclude client construction, page reads and
agent time. Time-to-five is first observed, including provisional snapshots;
a later error can retract them. `n` counts observed time-to-five values.

Decoded-byte sums are **observed lower bounds**, not compressed wire traffic.
An unobserved provider is not a zero-byte response. Overshoot is median retained
candidate count above five; it is not the final output count.

| Window | Clients | Policy | Search p50 / p95 ms | Time-to-five p50 ms (n) | Observed decoded bytes p50 | Overshoot p50 |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 1 | Fresh | full | 3003 / 3005 | 331 (31) | 137924 | 13 |
| 1 | Fresh | batch | 345.5 / 647 | 345 (32) | 130864 | 5 |
| 1 | Fresh | stream | 275 / 921 | 271 (31) | 81104 | 0 |
| 1 | Fresh | diversity | 3002 / 3004 | 272 (31) | 133984 | 12.5 |
| 1 | Reused | full | 3003 / 3004 | 211 (31) | 139062 | 24 |
| 1 | Reused | batch | 219 / 1282 | 219 (32) | 130861 | 5 |
| 1 | Reused | stream | 213.5 / 904 | 213 (32) | 81142 | 0 |
| 1 | Reused | diversity | 3002 / 3004 | 205.5 (32) | 134268 | 10 |
| 2 | Fresh | full | 3002.5 / 3005 | 313 (31) | 133263 | 5 |
| 2 | Fresh | batch | 346 / 3004 | 316 (29) | 130697 | 5 |
| 2 | Fresh | stream | 312 / 3003 | 303.5 (28) | 84201 | 0 |
| 2 | Fresh | diversity | 3002 / 3004 | 294 (30) | 131903 | 5 |
| 2 | Reused | full | 3003 / 3004 | 206 (30) | 133852 | 5 |
| 2 | Reused | batch | 224.5 / 3003 | 220 (28) | 130684 | 5 |
| 2 | Reused | stream | 244.5 / 1186 | 243 (32) | 81150 | 0 |
| 2 | Reused | diversity | 2070.5 / 3004 | 213 (29) | 134264 | 5 |

## Separately judged relevance and evidence

GPT-6 Codex manually inspected returned top-five titles, URLs and snippets,
unblinded, applying one shared judgment per query/URL across policies. There are
688 query/URL pairs. `true` means direct intent relevance; `false` means a wrong
entity/topic or only background, index or product information; three ambiguous
pairs remain null (EDUCBA authority, a C++-for-Rust article, and Tokio PR #6890).
Specialized ML methods are background to the general introduction; generic
runtime/database pages are background to method-specific questions. Metadata
relevance is not a claim about page accuracy or answer sufficiency.

Precision@5 includes missing slots as zero only for fully judged, error-free
calls; unknown-returned-URL calls are excluded and the denominator is shown.
No call returned an overall error, but successful empties remain in denominators.
Across all calls, missing final slots were full 24, batch 34, stream 25, diversity
29. Empty calls were respectively 4, 6, 5 and 5.

After both windows, 19 URLs were selected from their actual top-five union,
prioritizing references/explanations. At most three fetches per corpus query,
15-second HTTP / 30-second process timeout, 100000 characters, 4000000 decoded
bytes, no page cache. All argv, receipts, complete retained text and content hashes
are local. Eighteen extractions succeeded, but only fifteen contained the needed
explanation. Google MLCC was an index, the ownership chapter entry lacked the
mechanics of borrowing, and the GitHub PR capture lacked the relevant diff.
The PostgreSQL official page failed with “no extractable page text.” Wikipedia
ML reached its character cap, but its complete relevant definition was retained.
No successful extraction was automatically graded useful.

Verified coverage below means at least one of those fifteen supporting URLs in
a call's top five. Unfetched URLs remain unknown; **absence from the verified set
is not proof of no possible evidence**. Later page reads are a distinct snapshot,
not contemporaneous body rankings. Exact raw URLs were used for labels; tracking
parameters were collapsed only for human inspection of identical metadata.

| Window | Clients | Policy | Returned 5+ / 32 | Two retained providers / 32 | Mean direct P@5 (judged calls) | Verified support / 32 | Median top-five overlap vs full |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Fresh | full | 31 | 14 | 0.307 (30) | 21 | 1 |
| 1 | Fresh | batch | 32 | 0 | 0.200 (32) | 12 | 0.6 |
| 1 | Fresh | stream | 31 | 0 | 0.194 (32) | 11 | 0.6 |
| 1 | Fresh | diversity | 31 | 15 | 0.310 (29) | 19 | 0.8 |
| 1 | Reused | full | 31 | 17 | 0.323 (31) | 23 | 1 |
| 1 | Reused | batch | 32 | 0 | 0.200 (32) | 12 | 0.6 |
| 1 | Reused | stream | 32 | 0 | 0.200 (32) | 12 | 0.6 |
| 1 | Reused | diversity | 32 | 15 | 0.317 (29) | 20 | 0.7 |
| 2 | Fresh | full | 31 | 9 | 0.206 (32) | 15 | 1 |
| 2 | Fresh | batch | 29 | 0 | 0.181 (32) | 9 | 0.6 |
| 2 | Fresh | stream | 28 | 0 | 0.175 (32) | 8 | 0.6 |
| 2 | Fresh | diversity | 30 | 8 | 0.238 (32) | 15 | 0.6 |
| 2 | Reused | full | 30 | 10 | 0.258 (31) | 17 | 1 |
| 2 | Reused | batch | 28 | 0 | 0.181 (32) | 9 | 0.6 |
| 2 | Reused | stream | 32 | 0 | 0.200 (32) | 12 | 0.7 |
| 2 | Reused | diversity | 29 | 11 | 0.263 (32) | 18 | 0.6 |

Each following cell is verified support out of 16 calls (both windows and client
conditions). The source column identifies an actually fetched supporting page;
these are separate from the canonical gate and are never substituted into it.

| Corpus intent | Full | Batch | Stream | Diversity | Example supporting evidence |
| --- | ---: | ---: | ---: | ---: | --- |
| general | 16 | 16 | 16 | 16 | [IBM](https://www.ibm.com/think/topics/machine-learning): patterns learned from training data support predictions on new data. |
| science | 16 | 16 | 16 | 16 | [Solar panels](https://en.wikipedia.org/wiki/Solar_panel): illuminated PV cells produce excited electrons and direct current. |
| rust | 10 | 0 | 0 | 9 | [Rust reference/borrowing chapter](https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html): pass `&s1` instead of transferring the String; references do not take ownership. |
| postgres | 9 | 0 | 0 | 10 | [Neon EXPLAIN](https://neon.com/postgresql/postgresql-tutorial/postgresql-explain): ANALYZE executes the statement and reports actual node time/rows. The official PostgreSQL fetch failed. |
| long-tail | 3 | 0 | 0 | 3 | [Tokio Semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html): cancelling acquire loses its queue position; [select](https://docs.rs/tokio/latest/tokio/macro.select.html) lists it as not cancellation-safe. |
| fresh | 1 | 0 | 0 | 0 | [Rust 1.89 announcement](https://blog.rust-lang.org/2025/08/07/Rust-1.89.0/): exact version in text; date encoded in the official URL, not a retained dateline. |
| phrase | 9 | 0 | 0 | 7 | [Oracle](https://docs.oracle.com/en/java/javase/21/core/structured-concurrency.html): related tasks form one work unit; [Niebler](https://ericniebler.com/2020/11/08/structured-concurrency/) explains child completion before parents. |
| site | 12 | 10 | 11 | 11 | [Python TaskGroup reference](https://docs.python.org/3/library/asyncio-task.html): sibling cancellation and grouped ordinary failures. |

Matched live-call comparisons (128 pairs each) found full-only verified support
in 36 batch pairs and 36 stream pairs; the alternative alone had support in 2 and
3 pairs respectively. Diversity had 16 full-only and 12 diversity-only pairs.
The input responses differ between sequential calls: these are observed source
availability differences, not proof that truncating the exact same response lost
that source. Full and diversity were themselves unreliable on the exact-version
and long-tail queries. Provider failures and wrong-entity returns must be fixed
before inferring a generally preferable collection rule.

## Provider feasibility and cancellation

All nine providers were independently probed on all eight corpus queries with
fresh clients, the full policy and the same deadline/profile/pacing. Competing
providers and the result minimum cannot cancel an isolated probe. Timing cells
are p50 ms with observation counts; `—` is unobserved, never zero. The fields
aggregate logical searches across retries: first chunk and last recorded headers
can belong to different attempts. Do not subtract them as a per-response phase.

| Provider | Observed protocol / content type | Independent outcome (8 calls) | Headers (n) | First record (n) | Fifth unique (n) | EOF (n) | Records before observed EOF |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| duckduckgo | — | 8 deadlines; no headers | — | — | — | — | 0 |
| bing | HTTP/2; HTML | 7 results, 1 site-filtered empty | 222 (8) | 273 (7) | 273 (7) | 293 (8) | 7 |
| yahoo | HTTP/2; type absent | 8 errors; HTTP 500 observed in 4 | 777.5 (4) | — | — | 777.5 (4) | 0 |
| dogpile | HTTP/2; HTML error | 8 HTTP 403 | 121 (8) | — | — | 121 (8) | 0 |
| ecosia | HTTP/2; HTML error | 8 HTTP 403 | 189 (8) | — | — | 190 (8) | 0 |
| swisscows | HTTP/2; problem+json | 8 HTTP 429 | 337.5 (8) | — | — | 339.5 (8) | 0 |
| yep | HTTP/2; JSON | 1 results, 7 empty | 1358.5 (8) | 769 (1) | 769 (1) | 1358.5 (8) | 1 |
| qwant | HTTP/2; JSON error | 8 HTTP 403 | 221 (8) | — | — | 221 (8) | 0 |
| mojeek | HTTP/1.1; HTML error | 8 HTTP 403 | 167.5 (8) | — | — | 167.5 (8) | 0 |

Only the successful Bing/Yep independent responses establish pre-EOF record
delivery here. Errors' declared formats do not validate successful adapters.
Yahoo has four missing probe-map entries despite eight retained diagnostics.
All independent first-chunk/record/fifth/EOF values remain in `runs.jsonl`;
Bing first-chunk p50 was 229.5 ms (8), Yep 1358.5 ms (8). Compressed wire bytes,
server/intermediary buffering and per-attempt compression remain unmeasured.
Do not infer unsupported streaming from the other providers' blocks/deadlines.

The fanout observations include rare Qwant results absent in the independent
window. This is an availability difference, not contradictory parser evidence.
The following outcomes cover all 512 scheduled logical searches per provider;
`cancelled_min_results` may still have contributed a retained snapshot.

| Provider | Logical fanout outcomes (512 each) |
| --- | --- |
| duckduckgo | cancelled_min_results: 267, challenge: 49, deadline: 188, request_error: 8 |
| bing | cancelled_min_results: 113, filtered_empty: 63, results: 336 |
| yahoo | cancelled_min_results: 234, request_error: 278 |
| dogpile | cancelled_min_results: 2, request_error: 510 |
| ecosia | cancelled_min_results: 3, request_error: 509 |
| swisscows | cancelled_min_results: 143, request_error: 369 |
| yep | cancelled_min_results: 283, deadline: 7, empty: 140, results: 82 |
| qwant | cancelled_min_results: 17, request_error: 493, results: 2 |
| mojeek | cancelled_min_results: 9, request_error: 503 |

| Policy (128 calls) | Calls with deadline diagnostics | Exhausted without deadline/cancellation | Cancelled provider requests | Threshold-to-return p50 ms |
| --- | ---: | ---: | ---: | ---: |
| full | 110 | 18 | 0 | — |
| batch | 7 | 2 | 417 | 1 |
| stream | 5 | 0 | 547 | 1 |
| diversity | 67 | 12 | 107 | 1 |

Deadline/exhaustion counts are derived from retained diagnostics and are separate
from meeting the result minimum. Threshold-to-return measures only local
collector/fusion completion, not server acknowledgement or saved upstream work.
Production cancellation/pooling regressions remain covered by local transport tests.

## Resource measurements and remaining gaps

`/usr/bin/time -l` wrapped the optimized test executable, **not Cargo**. CPU
seconds and RSS below are whole-process observations, including client creation,
metadata hashing, all arms, serialization and pacing; they cannot isolate parser
CPU or attribute memory differences to a policy. RSS is bytes on this macOS host.

| Process | User CPU s | System CPU s | Max RSS bytes | Peak memory footprint bytes |
| --- | ---: | ---: | ---: | ---: |
| window-1 | 4.89 | 5.34 | 55574528 | 39109208 |
| independent | 0.51 | 0.39 | 51265536 | 34652712 |
| window-2 | 5.86 | 6.83 | 56606720 | 39813744 |

Independent Bing incremental-parser/worker elapsed p50 was 3826 μs (8 observed
calls); Yep was 16 μs (8). These include scheduling/waiting and are not CPU.
Zero/uninvoked parser fields are not used to claim zero parsing cost; final batch
parsing is excluded from that instrument. Per-arm isolated CPU/RSS, compressed
wire traffic, per-attempt protocol/compression/buffering and server-side
cancellation latency remain gaps. #29/#119 should supply compatible per-attempt
provider evidence; #80 owns process/deadline overhead attribution. Additional
independent windows, a broader held-out corpus and blind/body-complete judgments
remain required before closing #46 or changing defaults.

## Canonical ten-question gate

The [exact canonical dataset](../benchmarks/codex-search-2026-09-11/queries.json)
was run separately with the built CLI and its generated skill, installed into a
new temporary project. Assessor: GPT-6 Codex, manually applying the frozen
`AGENTS.md` minima. Every initial query was the exact manifest text. At most two
discovery calls and three direct fetches per question; all titles/URLs/snippets
were inspected, including wrong entities. Five recovery queries and their reasons
were saved before execution; they preserve entities, versions and site constraints.
No study URL or answer was used to fill a gate gap.

Exact discovery flags: `--no-fetch --no-rank -k 20 --min-results 20 --search-budget
10 --output json`. Fetch flags: `--output json --timeout 15 --content-limit 100000
--max-response-bytes 4000000`. Process bound 30 seconds; cache disabled; upstream
caches uncontrolled; remote telemetry disabled, local diagnostics/traces retained.
CLI redirects are automatic, but no official domain migration is claimed. The
learn.chatgpt.com pages were discovered and fetched on the requested domain.

Times below are summed monotonic subprocess wall seconds, including recovery;
search and fetch are separate, and total is their sum. They exclude agent reading
and reasoning. This acceptance observation is not a latency benchmark.

| ID | Judgment | Search s | Fetch s | Total s | Synthesized answer / missing evidence and reference |
| --- | --- | ---: | ---: | ---: | --- |
| q01 | **PASS** | 10.182 | 0.000 | 10.182 | Canberra is Australia’s capital; [Britannica snippet](https://www.britannica.com/place/Canberra) explicitly says “federal capital of the Commonwealth of Australia”. Fetch skipped because the entire fact is supported. |
| q02 | **PASS** | 10.163 | 0.425 | 10.588 | [Retrieved Rayleigh article](https://en.m.wikipedia.org/wiki/Rayleigh_scattering): shorter blue wavelengths scatter more strongly than longer red wavelengths, approximately λ⁻⁴, producing blue diffuse sky light. |
| q03 | **FAIL** | 20.380 | 0.000 | 20.380 | Ten third-party guides, then only Python documentation root/tutorial. No official TaskGroup failure/cancellation/ExceptionGroup/except* reference. Fetch skipped; [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148). |
| q04 | **FAIL** | 20.417 | 0.000 | 20.417 | Both site-restricted searches empty. No postgresql.org EXPLAIN ANALYZE/BUFFERS evidence; zero fetch attempts. [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148). |
| q05 | **FAIL** | 20.363 | 0.000 | 20.363 | Both site-restricted searches empty. No TraceQL parent/child operator or direction evidence; zero fetch attempts. [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148). |
| q06 | **PASS** | 10.158 | 0.621 | 10.779 | [Requested official reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents `otel.trace_exporter`, none/otlp-http/otlp-grpc and endpoint/headers/protocol fields. [Advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced) shows the `[otel]` configuration pattern. Supported TOML shown below. |
| q07 | **FAIL** | 20.356 | 0.000 | 20.356 | Ten unrelated Reddit results, then empty official-domain recovery. No official E0382 repair/ownership evidence; fetch skipped. [#32](https://github.com/rafaelpierre/kestrel-rs/issues/32), [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148). |
| q08 | **PASS** | 10.141 | 0.501 | 10.643 | [January 7, 2025 report](https://skyandtelescope.org/astronomy-news/trappist-1b-atmosphere-debated-some-stars-take-their-time-forming-planets/): airless resurfacing or less-likely hazy CO₂ atmosphere, not a detection. [NASA’s December-2025 retrospective](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/) says b may be bare rock and no thick atmosphere was seen. Planet e and April-2026 findings were not substituted. |
| q09 | **FAIL** | 20.303 | 0.000 | 20.303 | Generic Grafana sources, then empty Tempo 3.0 official-site recovery. No version-correct breaking-change or migration evidence; fetch skipped. [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148). |
| q10 | **PASS** | 2.010 | 0.430 | 2.440 | [Visit London museum entry](https://www.visitlondon.com/things-to-do/place/52747-science-museum): daily 10:00–18:00, last entry 17:15, galleries start closing 30 minutes before close; closed 24–26 December. Hours marked as supplied by the Science Museum. |

For q06, the documented fields can be expressed using TOML's nested-table syntax:

```toml
[otel.trace_exporter.otlp-http]
endpoint = "https://collector.example/v1/traces"
protocol = "binary"
```

Alternatively, `[otel]` with `trace_exporter = "none"` disables trace export.
`otel.exporter` is the separately documented event/log exporter. The reference
capture hit the character limit, but the complete trace-exporter field block was
retained. This gate checks Codex configuration evidence, not Kestrel telemetry.

**Gate NOT PASSED (5/10): q03, q04, q05, q07 and q09 fail.** No missing evidence
is waived as a baseline failure. Source assessment files retain synthesized
answers, exact supporting passages, chosen URLs, search/fetch disposition and
rationale. The failing searches still returned successful JSON, which is not an
evidence pass. A draft PR is required under the repository's acceptance policy.

## Reproduce and inspect

See [the experiment workflow](streaming-validation.md) for all four policies and
isolated probes. For each window use a new directory and the same environment:

```sh
KESTRELSEARCH_OTEL_ENABLED=false \
KESTREL_VALIDATION_TRIALS=4 KESTREL_VALIDATION_BUDGET=3 \
KESTREL_VALIDATION_CONTEXT='record region/network and host workload' \
KESTREL_VALIDATION_OUTPUT=benchmarks/results/NEW-window-1 \
cargo test --release --lib live_streaming_validation -- --ignored --nocapture
# Run live_streaming_provider_probes into a separate new directory, then window 2.
python3 benchmarks/streaming_validation_report.py \
  --input benchmarks/results/NEW-window-1 \
  --judgments PATH/TO/versioned-judgments.json \
  --output benchmarks/results/NEW-window-1/summary-judged.json
```

For comparable process resource measurements, first compile with `--no-run`, then
wrap the printed optimized test executable with `/usr/bin/time -l`; the exact argv
is retained in each receipt. The corpus's current passthrough contract supersedes
historical portable runs. Do not combine their schemas or tune policy after a
failed row. Each independent run refuses to reuse its output directory.

Full local artifacts are retained at
`/Users/rafaelpierre/projects/kestrel-rs-issue-46/benchmarks/results/`:
`streaming-46-window-{1,2}/`, `streaming-46-independent/`, resource stdout/stderr
and receipts, `tested.diff`, `study-fetch-plan.json`, `study-pages/`,
`study-judgments-v1.json`, `study-judgment-audit-v1.json`, `study-evidence-v1.json`,
`analysis-v1.json`, `gate-46-v1/`, and `gate-report-v1.json`. Local `inspect_*`,
`judge_study.py`, `analyze_study.py` and gate runner scripts preserve the exact
assessment/report workflow. The gate runner was reused from the in-progress
#148 worktree, copied locally with only its repository-root resolution adjusted;
its exact copy/hash is retained here. Raw responses, irrelevant/sensitive result
URLs and full captures are not published in this report.

Artifact SHA-256 identities:

- `streaming-46-window-1/runs.jsonl`: `bfe1ddc896c5c887be8b3e166ad1c44ac05e9101dc8a1a85c64ac1f9c163f7e9`.
- `streaming-46-independent/runs.jsonl`: `0e75960a060db3b3c420cd4e602b10153363cf76bb14fd12a4f37337b23cc8d7`.
- `streaming-46-window-2/runs.jsonl`: `2cb396245aade6ee66097c5e9325c7c99643fea1ad005aee1624986ac97dbae4`.
- `study-judgments-v1.json`: `080be9eee98901ed7952646337b80e5265174f280b62200b6cfea0d378fc4327`.
- `study-evidence-v1.json`: `e59e90df7fce1bca406c1fce72e7c48beeadeade37a6ebf5e1e902c7aa094e94`.
- `analysis-v1.json`: `1a372c4e816c5165c9406a629249abc0309846e893f34f875f8e8687ddea464c`.
- `gate-report-v1.json`: `747ddbfce81aa222919d84dff5c36d03cdc07bb6536798307e3ff1512e1b626e`.
- `gate_runner.py`: `59c6104d1494e6dd6ceb595444a2cb56d51123bf6b38db49a1a06f1341b3aad5`.

Validation: `cargo fmt --check`, Clippy all targets/features with warnings denied,
release build and five Python report tests passed. The first full Rust run failed
two existing deadline-phase assertions under concurrent load; its log is retained.
The unchanged full suite rerun passed **225 tests** (six live tests ignored),
including the new single-provider full-policy regression and existing streaming,
filtering/deduplication, deadline and HTTP/1.1/HTTP/2 cancellation coverage. This
matches the timing-sensitivity investigation in #124, not a fix claimed here.
Generated-skill temporary installation succeeded and its current workflow/help
was read and exercised by the gate. No public search/fetch/CLI contract changed,
so no skill-template update was required. CI and review status are reported in
the PR rather than inferred from these local checks.
