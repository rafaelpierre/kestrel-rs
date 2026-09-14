# Positive BM25 and RRF scaling (#23)

`examples/ranking_scaling.rs` exercises production snippet/hybrid ranking, RRF,
and the metadata fetch threshold on deterministic synthetic inputs without
networking. It uses 15/100/200/400 candidates, 200/2,000 tokens in each snippet
and body, ten query terms and 32 vocabulary terms. Hybrid reads both fields;
other lexical modes read metadata. Each condition excludes two warmups and
retains eleven samples. Percentiles use nearest rank (p50 sample 6, p95 sample
11); eleven samples are not a reliable population-tail estimate.

```sh
cargo build --release --locked --example ranking_scaling
KESTRELSEARCH_OTEL_ENABLED=false target/release/examples/ranking_scaling > ranking.jsonl
```

For a baseline comparison, copy the identical example into an isolated checkout
of `ae4d99ca682132336c7293cf6afd2f685529cf58` and build there. Preserve both
executables and their SHA-256 identities. Finish builds/tests before timing;
run baseline then optimized, followed by optimized then baseline. Retain all
runs and compare matched conditions. Input construction/cloning is outside the
timer; ranking/filtering and destruction of owned results are inside. Allocator,
CPU scheduling and other applications remain uncontrolled. These are CPU-stage
observations, not total live-search latency guarantees. No page/provider caches
or network requests are involved.

Positive scoring indexes unique query terms, scans each corpus token once, and
then scores with lookups in original query order. Expected hash-table time is
O(total corpus tokens + NQ), with O(NU + Q) auxiliary storage, where U is unique
query terms. Duplicate terms still contribute repeatedly; positive IDF, document
length normalization, signed empty sums and stable ordering remain unchanged.
The default body BM25 scorer and snippet pre-ranking are unchanged. RRF bypasses
all lexical scoring/tokenization and preserves its existing provenance semantics.

Regression tests compare score bits against the original quadratic reference,
including empty inputs, repeated/absent terms, Unicode and variable lengths;
metadata threshold tests check inclusive boundaries. RRF fixtures cover query
interleaving, stable ties, duplicate providers, foreign-query occurrences and
retention of original result fields. Generated skill guidance is validated with
the current Clap tree and temporary installation during the evidence gate.
