# Short-budget timing investigation (#80)

This harness measures the existing contract. It does not add a production flag,
change TLS verification, rename a budget, or promise a process deadline.
Run commands from the repository root, on an otherwise idle machine.

`prepare.py` copies source, tests, examples and the locked manifest into a new,
disposable directory. Exact-match source edits fail if the expected code changed.
Only that copy receives in-memory monotonic event markers, an explicit equivalent
Tokio runtime lifecycle, and a Bing/Yahoo endpoint override for local fixtures.
The observer emits its events **after** runtime shutdown. Never install this
benchmark binary or use it as a release artifact.

```sh
python3 benchmarks/budget-overhead/prepare.py target/budget-probe
cargo build --release --manifest-path target/budget-probe/Cargo.toml \
  --bin kestrel --example budget_retained --example budget_initialization
cargo build --release
python3 benchmarks/budget-overhead/run.py \
  --binary target/budget-probe/target/release/kestrel \
  --retained target/budget-probe/target/release/examples/budget_retained \
  --trials 10 --budget 1 --output benchmarks/results/budget-controlled
python3 benchmarks/budget-overhead/summarize.py benchmarks/results/budget-controlled
```

The three fixture conditions are:

- `hang`: Bing sends headers and an unfinished HTML prefix; Yahoo queues behind
  it at concurrency one. Both are cancelled at the shared deadline.
- `empty`: Bing completes with explicit no-results markup; Yahoo hangs. The CLI
  returns a valid empty result envelope.
- `page`: Bing completes with one result, below the default minimum of five;
  Yahoo hangs. Optional page fetching reads a local HTML page delayed by 50 ms.

Each condition has fetching disabled/enabled and diagnostics disabled/enabled.
CLI diagnostics-on enables existing provider traces and benchmark artifacts;
retained-client diagnostics-on enables provider traces (the library does not
write CLI artifacts). The CLI uses `--no-rank`; caching stays at its disabled
default. No-fetch/full-fetch comparison therefore isolates extraction without
mixing ranking or cache effects into it. Retained calls use the same clients for
all rounds; the summary separates round zero from subsequent calls. This is
client reuse, not a claim that every cancelled connection remains reusable.

Trials are sequential, with seeded shuffled CLI condition order in each round.
Retained conditions are sequential batches. No compilation or test workload
should overlap measurements. Raw stdout, stderr, existing artifacts/traces,
process launch/reap timestamps, binary hashes and environment metadata remain
under the ignored results directory. Output directories must be new.
`summarize.py` verifies return codes, result counts and fetched content for the
controlled cases before reporting distributions in milliseconds. With ten
observations, nearest-rank p95 is the maximum; it is not a precise tail estimate.

An interleaved observer-overhead control uses a one-nanosecond budget, which
expires before requests start. The original binary has no fixture override:

```sh
python3 benchmarks/budget-overhead/run.py \
  --binary target/release/kestrel \
  --compare-binary target/budget-probe/target/release/kestrel \
  --baseline --budget 0.000000001 --trials 20 \
  --output benchmarks/results/budget-observer-control
```

To isolate root-store initialization, run the initialization example in fresh
processes with `normal` and `preload`, alternating order for ten pairs. `preload`
calls primp's existing cached default-root-store API before constructing the
client, with the same native and bundled roots and unchanged verification. Each
process then constructs/drops ten clients. This moves work between measured
phases; it is not an optimization of total fresh-process time.

```sh
python3 benchmarks/budget-overhead/initialization.py \
  target/budget-probe/target/release/examples/budget_initialization \
  benchmarks/results/budget-initialization
```

For a separately labeled live reproduction, omit the endpoint override:

```sh
python3 benchmarks/budget-overhead/run.py \
  --binary target/budget-probe/target/release/kestrel \
  --live --query '"Sonic Youth" "tunings"' --trials 10 --budget 1 \
  --output benchmarks/results/budget-live
```

Live mode uses all default providers and concurrency, with the historical
`--min-results 1 --top-k 1` from #76. It cannot establish deterministic provider
availability or reproduce the historical binary. The timing experiment disables
ranking; the controlled fixture result minimum remains five.
Do not conflate changes in live results with a timing fix.

Boundary accounting uses within-process `Instant` differences and external
Python `perf_counter_ns` differences. Their origins are not assumed equal.
External duration minus `main_entry`→`runtime_dropped` is an explicitly
unpartitioned residual: loader/startup before Rust main, observer emission and
process exit after runtime shutdown, plus parent launch/reap overhead. Existing
artifact/lifecycle intervals are integer milliseconds; event markers retain
fractional seconds. Timer resolution does not imply scheduling precision.
The runner also records the parent's return from `Popen` to bound process-spawn
call overhead. Use `--no-fetch-only` for supplemental discovery boundary checks.

```sh
python3 -m unittest discover -s benchmarks/budget-overhead -p 'test_*.py'
cargo fmt --check --manifest-path target/budget-probe/Cargo.toml
cargo clippy --manifest-path target/budget-probe/Cargo.toml \
  --all-targets --all-features -- -D warnings
cargo test --manifest-path target/budget-probe/Cargo.toml --all-features
```

See [the investigation report](../../docs/issue-80-budget-overhead.md) for the
recorded environment, distributions, interpretation and limitations.
