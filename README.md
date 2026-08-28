# Kestrel Search for Rust

Kestrel Search is a keyless web-search, page-extraction, and relevance-ranking
tool for applications and AI agents. It searches DuckDuckGo, Bing, and Yahoo,
fetches candidate pages concurrently, extracts readable text, and ranks the
results with BM25.

This crate is the Rust port of the original Python `kestrelsearch` package. The
library crate is named `kestrelsearch` and the executable is named `kestrel`, so
the two implementations can coexist. Search flags, result JSON, provider
ordering, agent-skill locations, diagnostics, and benchmark artifacts remain
compatible with the Python implementation.

## Highlights

- No API key or hosted search service required.
- DuckDuckGo, Bing, and Yahoo in ordered fallback or concurrent fanout mode.
- Multiple queries with round-robin merging across query/provider buckets.
- Canonical-URL deduplication with provider and query provenance retained.
- Bounded concurrent downloads and HTML parsing, with response-size and
  extracted-content limits.
- BM25 ranking over extracted page content, with optional title/snippet
  pre-ranking before fetching.
- Optional provider quorum, total fetch budget, and TTL disk cache.
- Async and blocking library APIs, reusable HTTP connection pools, and detailed
  provider/page diagnostics.

## Install

Rust 1.89 or newer is required.

```bash
cargo install --path .
kestrel --help
```

## Use the CLI

The default command searches DuckDuckGo, fetches up to three times `--top-k`
candidates, extracts up to 2,000 characters per page, ranks them with BM25, and
returns the best five results:

```bash
kestrel search "python dataclasses"
```

Common variants:

```bash
# Machine-readable results
kestrel search "rust ownership" --output json

# Search results only: do not fetch pages or apply content BM25
kestrel search "openai news" --no-fetch

# Search several queries and providers concurrently
kestrel search "python typing" \
  --query "pyright docs" \
  --engine duckduckgo --engine bing --engine yahoo \
  --mode fanout

# Return after two providers per query produce results and cancel stragglers
kestrel search "python typing" \
  --engine duckduckgo --engine bing --engine yahoo \
  --mode fanout --provider-quorum 2

# Pre-rank title/snippet candidates before deciding which pages to fetch
kestrel search "rust async patterns" --pre-rank

# Reuse extracted pages for five minutes and cap total fetch latency
kestrel search "rust async patterns" \
  --cache-ttl 300 \
  --cache-max-entries 1000 \
  --fetch-budget 2
```

Progress is written to stderr and results to stdout, making `--output json`
safe to pipe into another program. Each JSON result can include `title`, `url`,
`display_url`, `snippet`, extracted `content`, `bm25_score`, primary
provider/query fields, and a `sources` list containing every deduplicated
occurrence.

Run `kestrel search --help` for all provider filters, concurrency controls, and
resource limits.

### Defaults and opt-in tradeoffs

Kestrel deliberately preserves the Python-compatible selection behavior by
default. Two measured optimizations remain explicit opt-ins:

- `--pre-rank` scores titles and snippets before page fetching. In a 24-pair
  live ablation it slightly reduced requests and downloaded bytes, but did not
  establish a semantic-quality improvement.
- `--provider-quorum N` avoids waiting for a slow provider after `N` providers
  per query return non-empty results. It improves fanout tail latency but may
  reduce provider diversity.

`--fetch-budget` is likewise an explicit latency/coverage tradeoff: pages that
finish within the total budget are retained and outstanding fetches are
cancelled. The cache is disabled unless `--cache-ttl` is supplied.

## Use the library

Reuse a `KestrelClient` across calls to retain its search and fetch connection
pools:

```rust,no_run
use kestrelsearch::{KestrelClient, SearchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = KestrelClient::new()?;
    let results = client
        .search_many(&["rust ownership".into()], &SearchOptions::default())
        .await?;

    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
```

The crate exports typed engine, mode, filter, search, and fetch options;
`search`, `search_many`, and `search_blocking`; bounded `fetch_all`; optional
`PageCache`; BM25 ranking helpers; and snippet candidate pre-ranking.

The corresponding `*_detailed` APIs return `SearchReport` and `FetchReport`
values. These report provider latency, retries, result counts and cancellations,
plus per-page queue, request/TTFB, download, parse, byte-count, cache, outcome,
and deadline data without changing normal result objects. Cache entries are
keyed by canonical URL and content limit so differently truncated extractions
cannot be mixed.

## Agent skill

Kestrel can install a `SKILL.md` that teaches Claude Code, Codex, or VS Code
Copilot when and how to invoke the CLI:

```bash
# Interactive target and scope selection
kestrel skill install

# Install for every supported agent in its global location
kestrel skill install --agent all --scope global

# Remove installations recorded by Kestrel
kestrel skill uninstall
```

Project installations use `.claude/skills`, `.codex/skills`, and
`.github/skills`. Installation records are kept in
`~/.kestrelsearch/config.toml`.

## Performance evidence

The development journal records three benchmark tracks: Python/Rust parity,
Rust optimization ablations, and Codex Web Search comparisons. Highlights from
the 2026-08-28 runs:

- Rust won 34 of 40 matched live CLI pairs after the first optimization pass,
  with a 12.3% paired median latency advantage in that run.
- CLI startup p50 was 5.3 ms for Rust versus 183.7 ms for Python in the initial
  comparison.
- A warm extracted-page cache reduced fetch p50 from a 1,895 ms cold prime to
  483 ms. A 500 ms total fetch budget held fetch p50 to 501 ms while retaining
  five enriched results in every trial.
- Against native Codex Web Search, Kestrel configured with three-provider
  fanout, quorum 2, and pre-ranking passed the same 21 of 24 semantic trials.
  Its end-to-end p50 was 15.6 s versus 21.6 s and its median non-cached token
  use was 7,114 versus 17,810.

These are live-network measurements, not universal performance guarantees.
See the [development journal](progress/2026-08-28.md),
[optimization plan](progress/PLAN.md), and [benchmark documentation](benchmarks/README.md)
for methodology, limitations, and result artifacts.

## Development status

Fine-grained diagnostics are complete. Snippet pre-ranking and provider quorum
have been evaluated and intentionally remain opt-in. The next planned work is
adaptive page-fetch scheduling, followed by per-host concurrency,
extracted-content deduplication, and batch/streaming operation.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

Network-facing tests use local mock servers. Live provider checks are manual
because provider markup, availability, and anti-bot behavior can change
independently of the crate.
