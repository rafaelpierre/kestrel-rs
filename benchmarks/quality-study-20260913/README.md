# Current-CLI latency and coverage pilot, 2026-09-13

This report addresses #75: correct the obsolete benchmark conditions, exercise
supported controls, and assess the resulting latency–coverage tradeoff. The
measurements use production search/fetch/ranking code from main `a8fe1e1` and
Kestrel 4.0.0. Main advanced through the 4.0.1 and 4.1.0 releases while the pilot ran; the
binary was frozen at the start of each batch. This is a pinned 4.0.0 measurement,
not a measurement of the newer extraction recovery or opt-in fetch-score gate.
The original tested runner/source snapshot is commit
`113b9e5c762a6c852361afae987793097e67a8ac`; the rebased runner additionally records
the new `min_fetch_score` default as disabled. Exact binary and source hashes accompany the
measurements. No production defaults were changed.

## Findings and recommendation

This pilot provides no basis to change the shipped defaults. It confirms a latency/coverage tradeoff when
shortening the deadline, but does not establish a generally superior policy.
It also exposes an important failed experiment: **62 of 63 live candidate-cap
runs had fewer than 15 available candidates, and none of the 21 matched triples
had comparable metadata inputs.** Those observations cannot identify the causal
effect of increasing the fetch cap. The local fixture independently verifies
that a fixed 15-result pool causes exactly 5/10/15 page requests, so the runner
and cap mechanism are exercised; the live fixed-pool acceptance criterion
remains outstanding on #75.

All 329 live calls exited successfully; there were no outer-process timeouts.
Valid empty responses therefore account for the empty rates below. The pilot
uses macOS 26.6.2 on arm64, Python 3.14.7, no proxy environment variables, and
one benchmark CLI process at a time. The frozen search binary SHA-256 was
`a457d5d921a5577a90488970775fac7a0badceb1f3b17e0b75a47a05c54e3d17`.

### Deadline sweep

Each row contains 35 calls across seven queries, with minimum five and top-k
five. Times are seconds. P@5 uses the declared metadata-relevance judgments.
The p95 values are exploratory observed order statistics.

| Search deadline | Process p50 / p95 | Command p50 / p95 | Empty | P@5 |
| --- | ---: | ---: | ---: | ---: |
| 1 second | 1.155 / 1.177 | 1.138 / 1.152 | 62.9% | 0.189 |
| 2 seconds | 2.148 / 2.169 | 2.133 / 2.145 | 48.6% | 0.269 |
| 3 seconds | 3.143 / 3.161 | 3.129 / 3.144 | 37.1% | 0.331 |
| CLI default, 5 seconds | 5.147 / 5.175 | 5.133 / 5.143 | 45.7% | 0.291 |
| No total deadline | 31.173 / 31.605 | 31.159 / 31.592 | 60.0% | 0.217 |

The one-second condition saves process time but loses judged relevant slots.
The three-second condition has the best pooled P@5 here, but its paired
query-bootstrap difference from five seconds spans approximately
**−0.006 to +0.091**. This does not establish a repeatable improvement. The
one-second difference spans −0.206 to −0.023, and the no-total-deadline difference
spans −0.189 to −0.006; these are exploratory intervals conditional on this small
query set and one assessor, with no multiple-comparison adjustment.

Per-query outcomes differ substantially: Broken Social Scene returned zero
results in all five no-total-deadline trials but 25 slots in the three-second
trials. The duet query returned 20 slots at both one second and no total deadline.
The coding query returned only one slot in the entire deadline sweep, and the
astronomy query returned none. These observations establish variability and
coverage limitations, not their provider-level causes (#76). No monotonic
coverage guarantee follows from extending the deadline.

### Minimum and fetching observations

| Condition | Calls | Process p50 / p95 | Command p50 / p95 | Empty | P@5 | Page-evidence slots |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Minimum 1 | 21 | 5.129 / 5.249 | 5.122 / 5.183 | 52.4% | 0.162 | 0% |
| Minimum 5 | 21 | 5.147 / 5.171 | 5.132 / 5.147 | 33.3% | 0.324 | 0% |
| Minimum 15 | 21 | 5.151 / 5.166 | 5.136 / 5.143 | 38.1% | 0.343 | 0% |
| Metadata, snippet ranking | 14 | 5.160 / 5.594 | 5.138 / 5.144 | 42.9% | 0.229 | 0% |
| Fetched, snippet ranking | 14 | 5.739 / 7.604 | 5.720 / 7.590 | 42.9% | 0.243 | 10.0% |

Lowering the minimum did not consistently improve latency in this pilot.
Fetching supplied some usable page text, but only 4 of 14 nonempty matched
query/round comparisons had identical selected metadata inputs. The pooled
fetching rows therefore describe live policies, not an isolated fetch effect.
The CLI has no metadata-only candidate cap; when collection overshoots, fetch
selection can introduce an additional difference. Empty pools are not treated
as affirmative comparability evidence.

A fetched Sonic Youth article retained an introduction but no concrete tuning
values within the 2,000-character limit. Another article pointed to a video for
the tuning details, which were absent from extracted text. The duet's official
homepage yielded only shopping-format text. These successful extractions earn
zero page-evidence coverage under the declared intents. Conversely, several
capital-city pages explicitly identify Canberra. Pages about Perth or Adelaide
as state capitals are off-topic for the national-capital question despite
matching the query terms. Evidence scoring audits the target fact, not every
incidental claim on those pages.

### Candidate-cap observations: comparability gate failed

| Fetch cap | Calls | Process p50 / p95 | Command p50 / p95 | Empty | P@5 | Page-evidence slots |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 5 | 21 | 5.756 / 6.927 | 5.741 / 6.907 | 38.1% | 0.286 | 18.1% |
| 10 | 21 | 5.520 / 6.571 | 5.502 / 6.552 | 28.6% | 0.314 | 14.3% |
| 15 | 21 | 5.552 / 8.190 | 5.544 / 8.170 | 42.9% | 0.305 | 8.6% |

Do not interpret these differences as effects of the cap: the input-pool gate
failed. A follow-up on #75 must run the cap comparison against a sufficiently
large frozen input pool. Increasing a live collection target alone does not
solve that experimental-control problem. Preserve these failed observations
and give the subsequent experiment a new version.

### Frozen ranking control

All five rankers were replayed on each of 28 frozen inputs. The table below
uses only the 14 inputs from the fetched condition; metadata-only inputs remain
separate in the [replay data](frozen-ranking.json). Each policy receives the
identical candidates and captured bodies within its input.

| Policy | Returned slots / 70 | Metadata P@5 | Page-evidence slots |
| --- | ---: | ---: | ---: |
| Provider | 37 | 0.286 | 18.6% |
| Snippet | 37 | 0.243 | 10.0% |
| Body | 28 | 0.200 | 21.4% |
| Hybrid | 37 | 0.300 | 10.0% |
| RRF | 37 | 0.286 | 18.6% |

The body policy returned fewer slots and more page-evidence slots in these
frozen cases; hybrid had the highest metadata P@5. This illustrates different
objectives, not a universally preferable ranker. #78 owns the fuller ranker
investigation. All five policies together took a median 0.0055 seconds and an
observed p95 of 0.0095 seconds per replay process (28 inputs); these are not
per-policy search-command timings.

The [compact measurements](data/measurements.jsonl), [aggregate summaries](data/summary.json),
[per-query summaries](data/per-query.json), [input checks](data/input-comparisons.json),
[versioned judgments](data/judgments.json), and [quality intervals](quality-uncertainty.json)
retain the negative results and make the analysis inspectable.

## Protocol

The seven queries are the four exact music queries from #76, the Australian
capital question (`q01`), Python TaskGroup exceptions (`q03`), and the 2025
TRAPPIST-1 b observations question (`q08`). This declared subset includes music,
general, coding and long-tail questions from `quality-queries-v1.json`. It is a
small exploratory set, not a held-out agent-answer evaluation.

The complete pilot runs 329 CLI calls: 63 minimum comparisons, 175 deadline
comparisons, 28 fetching comparisons and 63 candidate-cap comparisons. Each
stage uses a full condition-position cycle per query: 3, 5, 2 and 3 rounds,
respectively. Conditions rotate per query/round. Each new CLI process runs
sequentially, with a minimum 0.25-second idle pause between calls; provider,
fetch and parse concurrency are explicitly 10. Local page caching is off;
client pools are fresh per process. Upstream cache and provider state are
uncontrolled. The outer process timeout is 50 seconds and censors longer calls.

Minimum and deadline sweeps disable fetching and ranking, with top-k five.
The deadline sweep fixes the minimum at five. Fetching and candidate-cap sweeps
fix the minimum and nominal pool at 15. Fetching compares snippet ranking with
and without page retrieval, making the final ranking policy identical. Candidate
caps are 5/10/15, with hybrid ranking, pre-ranking off, and a five-second fetch
budget. Other effective options and each measured argv are in the export.

Live policy comparisons can change the collected inputs as well as the intended
treatment. Input comparisons explicitly check candidate availability and matched
metadata order; matching counts alone is insufficient. Frozen replay runs all
five rankers over the same captured candidates and content, isolating ranking
from a fresh retrieval. The replay example returns top five and measures all
five policies in one process; its process time must not be interpreted as a
per-policy command measurement.

## Judgment and uncertainty

The music intents are operational definitions recorded in the versioned query
set before measurement, not recovered answer keys from the original conversation.
For example, “Anthems” is ambiguous between the original song and a later tribute
album; this pilot treats song identification as direct and tribute-album pages
as background. Changing that intent would require a new judgment version.

A single unblinded Codex assessor reviews captured title/snippet metadata for
retrieval relevance and actual returned page text for evidence. Relevance is
0 (off-topic), 1 (background), or 2 (directly addresses the query intent).
Evidence is 1 only if the returned page text supports that intent; a successful
fetch or a relevant title does not suffice. Metadata-only records have no page
text and therefore no page-evidence coverage. This definition does not imply
that their snippets lack useful information. A future blinded multi-assessor
study could disagree with these judgments.

Judgments are keyed by query, URL and captured-content SHA-256. Unknown judgments
remain unknown. Precision@5 and page-evidence slot coverage include missing
slots as zero only when all returned results are judged. They do not measure
complete answer correctness, citation support, or agent/model latency (#84).

Process and command p50/p95 include all available observations. Outer timeouts
are censored, and unavailable command times remain missing. Query-cluster
bootstrap intervals use 1,000 resamples with seed 75; seven query clusters and
shared upstream conditions make the intervals exploratory. Each condition has
only 14–35 calls, so nearest-rank p95 is an observed order statistic, not a
reliable population tail estimate. Per-query regressions and empty/error
outcomes matter more than a favorable pooled number.

## Reproduction

From the issue branch, with a release binary built from the recorded production
source:

```sh
cargo build --release --locked
python3 benchmarks/run_quality_study.py --binary target/release/kestrel \
  --output benchmarks/results/issue-75-pilot-v1
cargo build --release --example rank_replay --locked
python3 benchmarks/replay_ranking.py --binary target/release/examples/rank_replay \
  --batch benchmarks/results/issue-75-pilot-v1/fetching \
  --output benchmarks/results/issue-75-rank-replay.jsonl
```

Choose a fresh output directory for a new experiment. A rerun samples a changing
network and is not expected to reproduce the same URLs or timings. The compact
export permits recomputing reported core metrics and checking hashes without
publishing raw fetched text or provider responses. Raw captures remain in the
ignored local benchmark directory. Public content hashes bind evidence
judgments to those captures; reproducing the semantic assessment requires the
captures or a new, separately labeled retrieval.


To verify the published core metrics and file checksums:

```sh
python3 benchmarks/verify_quality_export.py benchmarks/quality-study-20260913/data
python3 benchmarks/report_quality_uncertainty.py \
  benchmarks/quality-study-20260913/data/measurements.jsonl \
  --output /tmp/quality-uncertainty-new.json
```

To rebuild a compact export from the retained raw pilot and the published judgments:

```sh
python3 benchmarks/export_quality_study.py \
  --pilot benchmarks/results/issue-75-pilot-v1 \
  --judgments benchmarks/quality-study-20260913/data/judgments.json \
  --output benchmarks/results/issue-75-export-new
python3 benchmarks/report_rank_replay.py \
  --replay benchmarks/results/issue-75-rank-replay.jsonl \
  --batch benchmarks/results/issue-75-pilot-v1/fetching \
  --judgments benchmarks/quality-study-20260913/data/judgments.json \
  --output benchmarks/results/issue-75-ranking-report-new.json
```

## Validation and remaining work

The normal Rust suite includes a local streaming-provider fixture in which
minimum 1/5/15 arms collect different pools. The same 15-result pool and local
page content produce exactly 5/10/15 requests in the cap arms. Python tests
cover effective duplicates, CLI parsing without provider requests, invalid
cache/deadline combinations, counterbalanced positions, timing envelopes,
timeouts, malformed output, resume/attempt isolation, unknown judgments, pool
checks, export recomputation and tamper detection. Benchmark tests now run in CI.

The live fixed-pool candidate comparison remains incomplete, and a subsequent
current-version measurement must use the updated main binary rather than
relabeling this 4.0.0 pilot. Keep #75 open for
that work; this report makes no candidate-cap recommendation. #76 can use the
saved provider traces for causal attribution, #78 can extend the frozen ranking
analysis, and #84 can evaluate complete agent-answer workflows. Production
search/fetch/CLI contracts and the installed skill remain unchanged.
