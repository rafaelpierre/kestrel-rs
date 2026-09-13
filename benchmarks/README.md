# CLI comparison benchmark

For initialization, deadline and cleanup attribution, use the
[short-budget investigation harness](budget-overhead/README.md). It compares
controlled fresh processes with retained clients using an isolated benchmark
build; it does not change the shipped CLI.

`cli_compare.py` measures the Python and Rust CLIs without involving Codex. It
launches each matched Python/Rust query pair concurrently as independent
processes, records external wall time and Kestrel's search/fetch/rank artifact
timings, and runs a separate `--help` startup benchmark.
Rust artifacts also retain provider-level and page-phase diagnostics for
subsequent bottleneck analysis; older Python artifacts simply leave those
fields empty. The CLI comparison, quality/latency and query-semantics runners accept
both the current search JSON envelope (`results` plus `elapsed_seconds`) and legacy
result arrays. Stored benchmark schemas and external timing measurements are unchanged.

```bash
python3 benchmarks/cli_compare.py \
  --python-cli ../duckduckscrape/.venv/bin/kestrelsearch \
  --rust-cli target/release/kestrel \
  --trials 5 \
  --startup-trials 50 \
  --output benchmarks/results/cli-comparison.json
```

The default query set comes from the earlier eight-task web retrieval suite.

## Snippet pre-ranking ablation

`pre_rank_ablation.py` runs matched baseline and `--pre-rank` searches against
the same shuffled query sequence. Each pair runs concurrently to reduce bias
from changing provider/network conditions. The artifact reports phase latency,
requests, bytes, fetch outcomes, returned characters, and final URL overlap.

```bash
python3 benchmarks/pre_rank_ablation.py \
  --rust-cli target/release/kestrel \
  --trials 3 \
  --provider-quorum 2 \
  --output benchmarks/results/pre-rank-ablation.json
```

The script does not claim semantic quality from URL overlap. Use a separate
task-answer evaluation before changing the default candidate policy.

## Quality and latency investigation

`quality_latency.py` schema version 2 uses explicit current-CLI policies. The
versioned `quality-queries-v1.json` contains the four exact quoted music queries
from #76 plus ten general, coding and long-tail questions. Music additions are
exploratory; this is not the held-out agent-answer evaluation in #84.

```sh
python3 benchmarks/quality_latency.py --binary target/release/kestrel \
  --stage retrieval --dry-run --rounds 3
python3 benchmarks/quality_latency.py --binary target/release/kestrel \
  --stage search-budget --min-results 5 --rounds 5 --pace 0.25 \
  --trace --output benchmarks/results/budget-v2
python3 benchmarks/quality_latency.py --binary target/release/kestrel \
  --stage candidates --min-results 15 --candidate-pool 15 --rounds 3 \
  --output benchmarks/results/candidates-v2
```

The stages vary one factor at a time:

| Stage | Conditions | Fixed settings |
| --- | --- | --- |
| `retrieval` | Minimum 1, 5, 15 | No fetch/rank, same deadline |
| `search-budget` | 1, 2, 3 seconds, CLI default 5 seconds, no total deadline | Same minimum, no fetch/rank |
| `fetching` | Metadata vs fetched pages | Snippet ranking in both arms |
| `ranking` | Provider, snippet, body, hybrid, RRF | Same configured collection/fetch caps; live inputs can differ |
| `candidates` | Fetch caps 5, 10, 15 | Minimum/pool at least 15, pre-ranking off |
| `pre-rank` | Off/on | Same cap; separate from the candidate sweep |
| `concurrency` | Fetch concurrency 5/10 | Same other fetch settings |
| `fetch-budget` | 1, 2, 5, 20 seconds | Same search settings |
| `providers` | Each of nine engines separately | Same minimum and budget |

By default, enabled engines are explicitly all nine in CLI order; `--engine`
selects one instead. Outside its sweep, the minimum is 15 and the deadline is the
CLI default five seconds. Top-k and the candidate pool default to 5 and 15;
fetched arms explicitly use a five-second fetch budget, 2,000 extracted
characters, a 1,000,000-byte limit, and fetch/parse concurrency 10. Query syntax
is explicitly portable; native is a separately labeled diagnostic option. The
opt-in `--min-fetch-score` gate remains disabled and is recorded as null.
A high minimum is a stopping threshold, never an exhaustive-retrieval guarantee.
No total search deadline still leaves provider timeouts and the result minimum.
The outer process timeout is a censoring bound, not an unlimited measurement.

`--dry-run` emits effective conditions and planned commands; actual commands,
including per-run cache paths, are stored in each measured record. Equivalent
effective policies and incompatible cache/fetch overrides are rejected. Old
`fanout-q1/q2/all` and `search-0` labels are removed. Existing artifacts are not
rewritten and old batches cannot resume under schema 2. `--resume` checks the
binary, source/runner hashes, query selection, settings and captured environment.
Interrupted attempts are retained in separate directories and never mixed.
An invalid final record without a newline is treated as an interrupted append:
resume truncates only that incomplete tail and retries the run in a new attempt
directory. Complete records are retained, including a final JSON record missing
only its newline. Malformed newline-terminated records remain errors.

Calls are sequential (one CLI process), with a minimum idle interval of 0.25
seconds, including between warmup and measurement. Conditions rotate positions
for each query and reverse on alternate complete cycles. Use a multiple of the
number of conditions for position balance; metadata records incomplete cycles.
Use `--query-id` repeatedly to select a declared subset. All runs, including
errors, empty results and outer timeouts, remain in the output.

`--cache off|cold|warm` describes the extracted-page cache only. Cold runs get an
empty directory. Warm runs use a fresh directory and one saved, excluded warmup
for that query/condition; its exit status and actual cache hits are recorded.
Changing live URLs or a failed warmup may leave a measurement partly cold.
Metadata-only stages reject cache treatments. Upstream caches, network state and
other applications remain uncontrolled; proxy values/credentials are not saved.

Candidate checks record available and selected counts plus input hashes. The
report compares matched query/round metadata inputs (prefixes for cap sweeps).
An insufficient or missing pool is a failed comparability check, not evidence
that a larger cap is equivalent. Even equal counts do not mean equal inputs.
Live ranking/fetch arms are end-to-end policy observations; use frozen replay
for claims about ranking alone, and inspect pool equality before comparing them.

```sh
python3 benchmarks/quality_report.py benchmarks/results/budget-v2 \
  --judgments path/to/versioned-judgments.json --output path/to/report-v1.json
KESTREL_BENCH_TEST_BINARY="$PWD/target/release/kestrel" \
  python3 -m unittest discover -s benchmarks -p 'test_*.py'
cargo test --lib benchmark_minimum_arms_and_fixed_pool
```

Each batch writes a judgment template keyed by query, URL and captured-content
hash. Relevance is 0 (off-topic), 1 (background), or 2 (direct); evidence is 1
only if the captured page text supports the declared query intent. Supply a
rationale for each judgment. Missing judgments remain null, and content changes
invalidate evidence judgments. Evidence slot coverage is distinct from fetched
page counts and from complete answer correctness. The report includes process
and CLI command p50/p95, missing timing counts, timeouts, empty rates, candidate
and provider counts, fetch bytes/cache observations, and per-query results.
Query-cluster bootstrap intervals are exploratory: small query sets and shared
network state limit them, and small-sample p95 is not a reliable tail estimate.

For ranking comparisons on exactly the same retrieved content:

```sh
cargo run --release --example rank_replay -- PATH/TO/run-ARTIFACT.json
cargo run --release --example reuse_benchmark -- swisscows 'Rust E0382 use of moved value'
```

Do not promote a latency policy based on URL overlap or returned counts. First
review per-query precision@5, evidence coverage, version/date correctness and
whether a query lost its only direct result. Unknown URL judgments remain unknown.
Missing slots count as zero only in a fully judged batch. Keep old judgments and
results immutable; make corrections in a new version.

The [30-question fresh coding pilot](coding-pilot-20260911/README.md) is prepared;
agent trials remain gated on recovered retrieval quality. It has
10 development and 20 held-out questions, dated primary sources/atomic facts,
separate answer keys, and isolated no-tool/native/Kestrel trials with the same model
and budgets. Agent correctness, citation support and closed-book-failed results
must be recorded separately from retrieval relevance. No agent or native-search
measurements should be inferred from the subprocess runner.

Generated results, investigation reports, and captured provider responses are kept
locally and ignored by Git. Benchmark scripts and input datasets remain tracked.


The [pinned 4.0.0 CLI pilot for #75](quality-study-20260913/README.md) contains 329
sequential calls, frozen ranking controls, judged relevance/evidence, uncertainty,
and a reproducible compact export. Its live candidate-cap comparison failed the
input-pool gate; those rows are diagnostic observations, not causal cap estimates.
Reproduce the declared subset and matrix with `run_quality_study.py`.

## Current-contract streaming fanout validation

[Streaming validation](../docs/streaming-validation.md) compares deadline-bounded
full fanout, completed-batch stopping, current streaming stopping, and a
benchmark-only two-provider alternative. It uses `streaming-queries-v1.json`,
records fresh/reused pools and collector time-to-five, and reports paired latency,
coverage, canonical overlap and optional explicitly versioned relevance judgments.
The ignored live test writes only to a new, explicitly requested output directory;
`streaming_validation_report.py` produces a summary without overwriting evidence.
See the methodology for measurement gaps and the work still required by #46.
