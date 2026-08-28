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
