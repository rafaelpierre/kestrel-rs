# Kestrel Search for Rust

Kestrel Search is a keyless web-search, page-extraction, and relevance-ranking
tool for applications and AI agents. It searches DuckDuckGo, Bing, Yahoo,
Dogpile, Ecosia, Swisscows, Yep, Qwant, and Mojeek, fetches candidate pages
concurrently, extracts readable text, and ranks the
results with BM25.

This crate is the Rust port of the original Python `kestrelsearch` package. The
library crate is named `kestrelsearch` and the executable is named `kestrel`, so
the two implementations can coexist. Search flags, per-result fields, provider
ordering, agent-skill locations, diagnostics, and benchmark artifacts remain
compatible with the Python implementation, except for the CLI search JSON envelope
documented below.

## Highlights

- No API key or hosted search service required.
- All nine supported engines searched concurrently in fanout mode.
- Multiple queries with round-robin merging across query/provider buckets.
- Canonical-URL deduplication with provider and query provenance retained.
- Bounded concurrent downloads and HTML parsing, with response-size and
  extracted-content limits.
- BM25 ranking over extracted page content, with optional title/snippet
  pre-ranking before fetching.
- Result-count early stopping, total fetch budget, and TTL disk cache.
- Async and blocking library APIs, reusable HTTP connection pools, and detailed
  provider/page diagnostics.

## Install

### Apple Silicon macOS (recommended)

Install the latest prebuilt release:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/rafaelpierre/kestrel-rs/releases/latest/download/kestrel-rs-installer.sh | sh
```

The installer downloads and verifies the Apple Silicon binary from the latest
GitHub release. Intel Macs and Linux are not currently included in release
builds.

### crates.io

If Rust 1.89 or newer is installed, build and install the published crate with
Cargo:

```sh
cargo install kestrel-rs --locked
```

Verify either installation with `kestrel --version`. To install the optional
agent skill afterwards, run `kestrel skill install`.

These methods install for the current user, normally in `~/.cargo/bin` (or
`$CARGO_HOME/bin` when configured). Once that directory is on `PATH`, you can
run `kestrel` from any directory. The shell installer attempts to configure
`PATH`; follow its printed instructions to activate it in your current shell.
For a standard Cargo installation, sh/bash/zsh users can run
`. "$HOME/.cargo/env"`.

### Install a manually downloaded binary (macOS/Linux)

If you downloaded and extracted a release archive, run the executable from
the extracted directory:

```sh
./kestrel install                   # Current user: ~/.local/bin/kestrel
sudo ./kestrel install --system     # All users: /usr/local/bin/kestrel
./kestrel install --dir ~/bin        # Custom destination
```

Choose one of these installation scopes. The command copies the running binary,
creates the destination directory if needed, and prints shell-specific `PATH`
instructions. It does not edit shell configuration. For an all-user installation,
each user's `PATH` must include `/usr/local/bin`.

Running the installed copy's `install` command with the same destination is a
no-op. For other existing files and symlinks, the command asks `Replace it? [y/N]`.
Enter `y` or `yes` to replace the copy; any other answer or no input leaves it
untouched. Replacing a symlink leaves its target untouched. Prefer updating
package-managed installations with their package manager. The command also reports another
`kestrel` executable taking precedence on `PATH`.

`install` copies a binary you already have; it does not download releases or
add support for another platform. Prebuilt releases currently support Apple
Silicon macOS only. Linux users can build from source with Cargo.

## Use the CLI

The default command searches DuckDuckGo, Bing, Yahoo, Dogpile, Ecosia, Swisscows,
Yep, Qwant, and Mojeek concurrently, retaining
results from successful providers when others fail (including bot challenges).
It fetches up to three times `--top-k` candidates, extracts up to 2,000 characters
per page, ranks them with BM25, and returns the best five results:

```bash
kestrel search "python dataclasses"
```

To read a known page directly, use `fetch` instead of a `site:<full-url>` search:

```bash
kestrel fetch "https://www.rust-lang.org/learn"
kestrel fetch "https://www.rust-lang.org/learn" --output json --content-limit 40000
```

`fetch <URL>` extracts one HTTP/HTTPS page without search or ranking. It defaults
to 20,000 characters, a 10-second timeout, and a 1 MB (1,000,000-byte) response
limit (adjust with `--content-limit`, `--timeout`, and `--max-response-bytes`).
JSON output is an
object with `url`, `content`, and numeric `elapsed_seconds` fields. Failed requests, unsupported content such
as PDFs, and empty extractions exit unsuccessfully with an error on stderr.
At `--max-response-bytes`, fetching stops and the retained prefix is extracted
instead of rejecting the page, even if its declared size exceeds the cap. The cap
applies to decoded body bytes, independently of the extracted character limit.
Usable partial content succeeds with the existing output schema and a notice on
stderr; a prefix without extractable text still fails. Reaching the cap exactly
is conservatively treated as potentially incomplete. Increase the byte cap to
retrieve more of a large page. Use `--max-response-bytes 2000000` to restore the
previous 2 MB allowance. This also applies to fetched search candidates; search
reports the number of successfully extracted pages that reached the cap on stderr.
Search-provider response limits are unchanged. Byte-capped extractions are not
cached. The extractor does not render JavaScript.

HTML extraction preserves document order and inline word boundaries, including
short answers, nested lists, code and tables. Blocks use line breaks, table cells
use tabs, and `pre` retains code indentation and repeated lines. Headings are
included once per source occurrence without an implicit BM25 boost. Character
limits count retained whitespace and separators; legacy unversioned cache entries
are invalidated by the conservative, versioned page-cache identity. See the
[HTML extraction contract](docs/page-extraction.md#ordered-readable-text-issue-22).

Page extraction removes structural chrome (such as navigation and sidebars)
and explicit clutter markers using whole class tokens and scoped ID names.
Names such as `download`, `reader`, `shadow`, and `thread` retain their content;
arbitrary substrings are not treated as advertisements. Both direct fetch and
search candidate fetching use this rule. See the
[extraction contract and fixture evidence](docs/page-extraction.md).
If the chosen HTML body is entirely recognized shell messages, extraction may
recover text from the first explicit article. Content-quality assessment remains
advisory: library methods and opt-in search artifacts identify known shell-only
text, mixed/unknown text, or text with no known shell indicators. No state
certifies usefulness. Fetch still returns flagged text successfully, and ranking
still uses it; normal CLI JSON and flags are unchanged. See the
[quality policy and evaluation](docs/content-quality.md).

Direct fetch and search candidate fetching support `text/plain` as well as
HTML/XHTML. Plain text is decoded using the declared supported charset (UTF-8
when absent or unrecognized) and limited by Unicode characters, preserving line
breaks, indentation, repeated lines, and literal markup/entities such as `<p>`
and `&amp;`. HTML cleanup applies only to HTML/XHTML; responses without a content
type retain the HTML fallback. Invalid byte sequences, including a multibyte
character split by the byte cap, decode with replacement characters. Empty or
whitespace-only retained plain text has no extractable content.

Fanout is the only search mode. `--mode fanout` remains accepted for compatibility;
`--mode fallback` is no longer supported. In the library, `SearchMode::Fanout`
is the default and only variant; migrate uses of `SearchMode::Fallback` to it.

Common variants:

Search runs all nine default providers concurrently. Use
`--engine` to select specific providers. For example,
`kestrel search "test" --engine bing --no-fetch` searches only Bing.
Use `-e duckduckgo -e bing -e yahoo` to retain the previous default provider set.
The library’s `SearchOptions::default()` uses the same nine-engine order.
More engines increase provider requests; existing budgets and concurrency limits
still apply. Availability and region/recency filter support vary by provider.

```bash
# Machine-readable results
kestrel search "rust ownership" --output json

# Search results only: do not fetch pages or apply content BM25
kestrel search "openai news" --no-fetch

# Search several queries and providers concurrently
kestrel search "python typing" \
  --query "pyright docs" \
  --engine duckduckgo --engine bing --engine yahoo

# Bound provider search to three seconds and stop at five accepted candidates
kestrel search "rust ownership" \
  --min-results 5 --search-budget 3 --no-fetch

# Return after five unique results per query and cancel unfinished requests
kestrel search "python typing" \
  --engine duckduckgo --engine bing --engine yahoo \
  --mode fanout --min-results 5

# Pre-rank title/snippet candidates before deciding which pages to fetch
kestrel search "rust async patterns" --pre-rank

# Reuse extracted pages for five minutes and cap total fetch latency
kestrel search "rust async patterns" \
  --cache-ttl 300 \
  --cache-max-entries 1000 \
  --fetch-budget 2
```

Provider search stops at `--min-results` unique accepted candidates per query
(default five), or when providers finish or the search budget expires. This is
independent of `--top-k` (the final result ceiling) and `--fetch-candidates`
(the page-candidate ceiling, default three times top-k). To rank a larger pool,
raise both the collection threshold and fetch limit, for example
`kestrel search '"machine learning"' --min-results 15 --fetch-candidates 15 -k 5`.
Neither collection nor fetching guarantees five final results: requests can fail
and BM25 can filter candidates. Kestrel does not refill failed fetch slots.

Use optional `--min-fetch-score SCORE` to reject weak title/snippet candidates
before the fetch cap. This positive-IDF BM25 gate runs even for small pools and
without `--pre-rank`; it removes rejected candidates from final output as well
as fetching. It is disabled by default and requires fetching.
Scores are finite nonnegative numbers with an inclusive cutoff (zero keeps zero
scores), and depend on the candidate pool; no universal nonzero cutoff is
recommended. For example, `kestrel search "rust async" --min-results 15
--fetch-candidates 8 --min-fetch-score 0.1 --no-rank` illustrates syntax, not a
calibrated threshold. Queries without lexical terms bypass the gate;
all-rejected searches return empty without page/cache work or automatic refill.
See [argument responsibilities](docs/cli-arguments.md#optional-fetch-score-threshold)
for multi-query handling, diagnostics and interactions. Regenerate installed
skills using the updated binary.

`--provider-quorum` is retained for compatibility but ignored by result-count
fanout. `--search-budget` covers provider search; page fetching has its own
`--fetch-budget`. Use `--no-fetch` when only search results are needed.

Successful `search` and `fetch` commands print elapsed wall-clock seconds to three
decimal places on stderr, for example `[kestrel] Search completed in 1.234 seconds.`
or `[kestrel] Fetch completed in 0.125 seconds.` This includes initialization,
retrieval, extraction, optional ranking, and result output, but excludes CLI
argument parsing and process startup. Empty successful searches also report time.
Both text and JSON modes use the same timing diagnostic; text output is unchanged.
Failed commands retain their error diagnostics without a success completion line.

Progress is written to stderr and results to stdout, making `--output json`
safe to pipe into another program. Each JSON result can include `title`, `url`,
`display_url`, `snippet`, extracted `content`, `bm25_score`, primary
provider/query fields, and a `sources` list containing every deduplicated
occurrence.

JSON output includes default structured `diagnostics` (`schema_version: 1`) and
command-level `elapsed_seconds`, a finite, nonnegative number that preserves
fractional seconds without rounding to three decimal places.

With `--no-diagnostics` (the previous envelope):

```json
{"results": [], "elapsed_seconds": 0.125}
```

Search always returns an object, including when `results` is empty. With
`--no-diagnostics`, fetch returns
`{"url": "https://example.com/page", "content": "Source: ...", "elapsed_seconds": 0.125}`.
The JSON timer uses a monotonic clock from command-handler entry through client
initialization, retrieval, extraction and optional ranking. It is sampled before
JSON serialization/output, so it can differ from the final stderr timing. Process
startup and argument parsing are excluded. Errors retain their existing exit status
and stderr diagnostics without a JSON error envelope.

**Default diagnostics migration:** both JSON commands now add a bounded
`diagnostics` object with completion conditions, stage counts and advisory page
evidence. Add `--no-diagnostics` to restore the previous envelopes for strict
consumers. Text output and error/exit behavior are unchanged. See the
[structured diagnostics contract and retry examples](docs/structured-diagnostics.md).

**Breaking JSON migration:** search previously returned a top-level array; read
`.results` now. For example, change `jq '.[]'` to `jq '.results[]'`, or Python
`json.loads(stdout)` to `json.loads(stdout)["results"]`. The opt-out does not restore
the historical root array. Library result types and benchmark artifact schemas
are unchanged.

Run `kestrel search --help` for all provider filters, concurrency controls, and
resource limits. See the [argument interaction reference](docs/cli-arguments.md)
for stage boundaries and compatible combinations.

Numeric inputs are checked before requests: durations must round to at least one
nanosecond and fit a monotonic deadline; concurrency cannot exceed Tokio's
semaphore capacity. Overflowing default candidate counts require a smaller
`--top-k` or explicit `--fetch-candidates`. Invalid CLI values exit with status 2;
library search/fetch options return `InvalidRequest`. See the
[numeric boundaries](docs/cli-arguments.md#numeric-boundaries).

Explicit fetch-stage settings (such as `--timeout`, `--pre-rank`, and cache
options) now conflict with `--no-fetch` instead of being silently ignored.
Choose at most one of `--rank`, `--no-rank`, or `--ranking-policy`; explicit
`--rank` and body policy require fetching. Conflicts exit with usage status 2
before requests. Remove redundant or inactive settings from existing scripts.
`--no-fetch --no-rank` and metadata policies without fetching remain valid.

### Defaults and opt-in tradeoffs

CLI fanout searches have a five-second total search budget, including provider
queueing and retries. Completed results are retained when the deadline expires.
Use `--search-budget SECS` to change it or `--no-search-budget` to disable the total deadline
while retaining result-count early stopping. The library has no total search deadline
unless one is supplied. Per-request timeouts still apply.
This budget does not include page fetching or ranking.

Search requests, page fetching, and HTML parsing each default to a maximum
concurrency of 10 in both the CLI and library. Override them with
`--search-concurrency`, `--concurrency`, and `--parse-concurrency`, respectively.

Two measured optimizations remain explicit opt-ins:

- `--pre-rank` scores titles and snippets before page fetching. In a 24-pair
  live ablation it slightly reduced requests and downloaded bytes, but did not
  establish a semantic-quality improvement.
- `--min-results N` controls the unique-result target for fanout.
  `--provider-quorum` is accepted for compatibility but ignored.

`kestrel search "machine learning" --mode fanout` now stops each query after **five
valid, unique search candidates**. Change the target with `--min-results N`.
Challenges, failed requests, empty responses, invalid URLs, and filtered-out
records contribute zero. Duplicate URLs count once. There is no separate
streaming flag and no requirement to wait for every provider.

All adapters publish the same `SearchResult` contract: title, URL, display URL,
snippet, provider, query, original provider rank, and source provenance. HTML
adapters emit closed result cards; JSON adapters emit complete result items.
Providers with encoded envelopes or unrecognized incremental layouts deliver a
completed batch through the same collector. Once the target is reached, pending
requests for that query are cancelled and their unread tails are ignored. Shared
HTTP/2 connections remain available for other searches.

One provider can supply all five valid results, even if `--provider-quorum 2`
is supplied. Fusion combines whatever has arrived without imposing a minimum
number of providers. The result minimum
is not an output cap or a promise of five successfully fetched pages: a chunk
can contain extra candidates, and exhausted providers or the search deadline
can return fewer. Complete streamed records survive a deadline. Existing fusion
and ranking apply to the retained candidates.

`SearchOptions::min_results` controls the library target; `None` uses five in
fanout mode. External struct literals must add the field or use
`..Default::default()`. See [streaming behavior and measurements](docs/streaming-fanout.md).

`--fetch-budget` is likewise an explicit latency/coverage tradeoff: pages that
finish within the total budget are retained and outstanding fetches are
cancelled. With caching enabled, the same absolute deadline includes cache reads,
writes and bounded maintenance; returned text survives a persistence timeout.
Blocking disk operations already admitted may finish after cancellation, bounded
to four per cache instance and its clones. See [cache deadlines](docs/cache-deadlines.md).
The cache is disabled unless `--cache-ttl` is supplied. Eligible pages commit while
other fetches run; a restarted search can reuse committed text even if the prior
process was killed. Discovery repeats unless provider recovery is also enabled. See [incremental page commits](docs/incremental-page-cache.md)
for durability, bounded storage and compatibility limits.

The [interrupted-search recovery contract](docs/cache-recovery-contract.md)
records current cache limitations and the proposed delivery interfaces. Provider
progress recovery is not yet implemented.

## Use the library

Reuse a `KestrelClient` across calls to retain its search and fetch connection
pools and aggregate page parser capacity:

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

Clients default to 10 queued/running page parsers across all calls and clones.
Use `KestrelClient::with_parser_capacity(n)` (or
`with_transport_and_parser_capacity(transport, n)`) to configure this shared cap.
Each batch also obeys `FetchOptions::parse_concurrency`; larger per-call values
do not raise the client cap. For library callers previously using values above
10, explicitly configure a larger client capacity to retain that concurrency.
Cancellation and fetch-budget expiry return without waiting for blocking parsers,
but their slots remain occupied until the body/DOM is released. Runtime shutdown
can still wait for those jobs. Free fetch functions and independently constructed
clients own separate capacity. See [ownership boundaries](docs/page-extraction.md#parser-capacity).

The crate exports typed engine, mode, filter, search, and fetch options;
`search`, `search_many`, and `search_blocking`; bounded `fetch_all`; optional
`PageCache`; BM25 ranking helpers; and snippet candidate pre-ranking.

The corresponding `*_detailed` APIs return `SearchReport` and `FetchReport`
values. These report provider latency, retries, result counts and cancellations,
plus per-page queue, request/TTFB, download, parse, byte-count, cache, outcome,
and deadline data without changing normal result objects. Cache entries are
keyed by conservative request URL, extraction version and content limit so differently truncated extractions
cannot be mixed.
Page keys preserve trailing slashes, encoded paths and every query parameter
(including order and tracking parameters), while ignoring URL fragments. Legacy
unversioned entries are misses and are not migrated. Search deduplication is
unchanged. See [page-cache identity](docs/page-cache-identity.md).

## OpenTelemetry / Honeycomb

Set an OTLP endpoint to export hierarchical search/fetch traces. The maintained
test runner attributes each Rust/Python test and its CLI subprocesses; content
capture is opt-in and bounded. See [configuration, Honeycomb setup and test
coverage](docs/telemetry.md). Ordinary credential-free tests remain available.

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

The generated skill begins with a bounded discovery, source inspection, selective
reading and stopping workflow. It distinguishes fast metadata lookup from page
evidence needed for details or quotations, and documents installation and optional
diagnostics. See the [skill capability audit](docs/skill-capabilities.md) for coverage
and capabilities that remain outside the CLI.

Project installations use `.claude/skills`, `.codex/skills`, and
`.github/skills`. Installation records are kept in
`~/.kestrelsearch/config.toml`. Updates use a persistent `config.toml.lock`
sidecar and atomic replacement to protect records from concurrent Kestrel commands
and interrupted writes. Lock contention fails after ten seconds; retry once the
other installation finishes. Do not delete the lock file. See
[installation-state guarantees and limits](docs/installation-state.md).

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

Fine-grained diagnostics are complete. Snippet pre-ranking remains opt-in;
provider quorum is superseded by result-count stopping. The next planned work is
adaptive page-fetch scheduling, followed by per-host concurrency,
extracted-content deduplication, and batch/streaming operation.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

### Releases

Releases use Conventional Commit messages to select the next semantic version:

- `fix:` creates a patch release.
- `feat:` creates a minor release.
- `feat!:` or a `BREAKING CHANGE:` footer creates a major release.

After changes reach `main`, release-plz creates or updates a release PR containing
the version and changelog. Merging that PR creates a `vMAJOR.MINOR.PATCH` tag.
The tag runs the generated cargo-dist workflow, which builds the Apple Silicon
binary and publishes it, its checksum, and `kestrel-rs-installer.sh` to a
GitHub Release.

The workflows use repository secrets for crates.io and GitHub release
publishing and explicitly dispatch cargo-dist at the new tag.

Network-facing tests use local mock servers. Live provider checks are manual
because provider markup, availability, and anti-bot behavior can change
independently of the crate.

### HTTP/2 connection tuning

Retain and clone `KestrelClient` to reuse connections across calls. Shared transport
settings, opt-in origin warm-up, DNS caching, protocol diagnostics and a cold/warm
benchmark are described in [HTTP/2 transport tuning](docs/http2.md).

### Experimental providers and latency controls

Default adapters include Dogpile, Ecosia, Swisscows, Yep, Qwant and Mojeek.
Form queries as concise keyword-based full-text search (FTS) terms. Agents must
NEVER submit conversational questions or semantic prompts: translate
`why is the sky blue Rayleigh scattering` to `Rayleigh scattering blue sky`.
Preserve the user's intent, entities, technical identifiers, phrases, negation,
domains, versions and dates when translating. The generated skill teaches this
rule for initial and recovery queries; the CLI does not rewrite or reject prose.

Search defaults to provider-native query passthrough. Shell quotes in
`kestrel search "machine learning"` group one argument; they do not add local
AND or phrase constraints. Literal quotes in `kestrel search '"machine learning"'`
are sent to providers unchanged, with provider-dependent phrase semantics.
Results are not rejected merely because title/snippet metadata omits query terms.
Existing native hostname restrictions and HTTP(S) URL validation remain.

Portable mode and `--query-syntax` have been removed from the CLI; the Rust
`QuerySyntax` enum and `SearchOptions.query_syntax` field are also removed.
Remove these options from existing commands and callers. No local Boolean/phrase
filter replaces them. The optional `--min-fetch-score` gate now scores tokenized
query text without Boolean parsing, like lexical ranking; site/exclusion-only
queries no longer automatically bypass that gate. Provider retrieval quality and
latency are not guaranteed by this change.

See [provider contracts, query syntax, randomized headers and pooled HTTP/2 transport](docs/search-providers.md)
and the [quality/latency benchmark workflow](benchmarks/README.md).

Provider work can be recovered independently with `--recovery-ttl 300`,
`--recovery-dir ./progress` and optional `--recovery-max-entries 1000`, including
metadata-only searches. Repeat the same command and directory after interruption to replay committed records
and request only necessary incomplete units.
See [provider progress storage](docs/provider-progress.md) for initial/retry recipes, commit boundaries and graceful shutdown.
