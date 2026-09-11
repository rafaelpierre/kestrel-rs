# CLI comparison benchmark

`cli_compare.py` measures the Python and Rust CLIs without involving Codex. It
launches each matched Python/Rust query pair concurrently as independent
processes, records external wall time and Kestrel's search/fetch/rank artifact
timings, and runs a separate `--help` startup benchmark.
Rust artifacts also retain provider-level and page-phase diagnostics for
subsequent bottleneck analysis; older Python artifacts simply leave those
fields empty.

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

`quality_latency.py` runs sequential, alternating conditions for three rounds,
records the exact binary hash/version, Git commit and tracked diff hash, arguments,
monotonic process timing, failures, candidate artifacts and optional provider traces.
It refuses to overwrite a batch; `--resume` requires matching metadata.

```sh
python3 benchmarks/quality_latency.py --binary target/release/kestrel \
  --stage retrieval --search-budget 3 --trace --output benchmarks/results/retrieval-new
python3 benchmarks/quality_latency.py --binary target/release/kestrel \
  --stage providers --search-budget 3 --trace --output benchmarks/results/providers-new
```

The common deadline is explicit: do not combine these numbers with unrestricted
historical calls. Process latency is not native Web Search complete-call latency.
`--stage swisscows` measures a single usable provider; subsequent stages are
`ranking`, `candidates`, `concurrency`, `fetch-budget` and `search-budget`.
Use `--engine` to select a recovered provider for fetch experiments.
`--cache cold|warm` makes local page-cache state explicit (upstream state remains
uncontrolled); warm runs have an excluded prewarming call per measured query.

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

Investigation findings and completed measurements: [September 11 implementation report](investigation-2026-09-11/REPORT.md).
