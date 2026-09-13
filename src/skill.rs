//! Agent skill content generated from the live Clap command tree.

use std::fmt::Write;

use clap::Command;

const HEADER: &str = r#"---
name: kestrelsearch
description: >
  Multi-engine web search with BM25 relevance ranking and direct URL fetching.
  Use when asked to search the web, look something up, find recent information,
  research a topic, browse the internet, or read a specific webpage URL.
  Trigger phrases: search the web, look up, find information about, google,
  browse the web, web search, find recent, what is the latest on.
argument-hint: "search <query> | fetch <url>"
---

# Kestrel Search

Kestrel Search — web search, page extraction, and relevance ranking for AI agents.

## Formulate keyword FTS queries

Always translate the user's request into a concise keyword-based full-text search
(FTS) query before calling `search`. NEVER submit conversational questions or
semantic prompts as search queries. Use the terms likely to appear in the source:
subject names, technical identifiers and the needed property or relationship.
Remove question scaffolding such as "why is", "what is" and "how do I"; preserve
meaning-bearing terms, requested phrases, negation, site restrictions, versions
and dates. Preserve exact error messages such as `use of moved value`; words
inside an error message are search terms, not question scaffolding. Apply this rule to every `--query` and every recovery search.

| User request or unsuitable query | Keyword FTS query to send |
| --- | --- |
| `why is the sky blue Rayleigh scattering` | `Rayleigh scattering blue sky` |
| `What is the capital of Australia?` | `Australia capital` |
| `Rust E0382 use of moved value how to fix` | `Rust E0382 use of moved value` |
| Find official PostgreSQL EXPLAIN ANALYZE buffer documentation | `site:postgresql.org EXPLAIN ANALYZE BUFFERS` |

FTS here describes how the agent writes the query. Kestrel passes query text to
providers unchanged; it does not rewrite questions, perform semantic expansion,
or enforce a local Boolean/phrase language. Provider operator support varies.
Keep the user's full question as the answer goal and judge retrieved evidence
against that goal; shortening a query must not relax its requirements.

"#;

const SCHEMA_AND_NOTES: &str = r#"
## Search JSON output schema

`search --output json` returns an object with `results` (an array, empty when no
results are found), `elapsed_seconds` (a finite, nonnegative number in seconds),
and default `diagnostics` (`schema_version: 1`). Add `--no-diagnostics` to restore
the previous envelope. Example with `--no-diagnostics`: `{"results": [], "elapsed_seconds": 0.125}`.
This is a breaking change from the previous top-level array. Read `.results`
instead of the root array (for example, migrate `jq '.[]'` to `jq '.results[]'`).
The opt-out restores the previous object, not the historical root array; library result types are unchanged.
Each result in `results` has
`title`, `url`, `display_url`, `snippet`, and `content`; optional fields are omitted
when unavailable, rather than serialized as null:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Page title |
| `url` | string | Full canonical URL |
| `display_url` | string | Shortened URL shown by the search engine |
| `snippet` | string | Search-result snippet |
| `content` | string or null | Extracted main-body text prefixed with `Source: <url>` |
| `bm25_score` | number, optional | Content-only BM25 score; experimental snippet/hybrid/RRF scores are not exposed |
| `engine` | string, optional | Engine that supplied the retained result |
| `query` | string, optional | Query that supplied the retained result |
| `engine_rank` | integer, optional | Original provider/query position |
| `sources` | array, optional | Provider/query occurrences merged into this URL; omitted when empty |

Each `sources` entry is an object with `engine` (string), `query` (string), and
`rank` (integer, original one-based provider position). `content` is null when
page text is unavailable, including searches with `--no-fetch`.

## Notes

- This `SKILL.md` is compatible with Claude Code, Codex, and GitHub Copilot in VS Code.
- Progress logs go to **stderr**; use `--output json` for machine-readable **stdout**.
- Successful `search` and `fetch` commands report elapsed wall-clock seconds to three decimal places on stderr, e.g. `[kestrel] Search completed in 1.234 seconds.` or `[kestrel] Fetch completed in 0.125 seconds.` This includes initialization, retrieval, extraction, optional ranking, and result output; it excludes argument parsing and process startup. Empty successful searches also report time. Text output is unchanged, and failures do not print a success completion line.
- JSON `elapsed_seconds` uses a monotonic clock from command-handler entry through initialization, retrieval, extraction and optional ranking. It is captured before JSON serialization/output, so it may differ from the final stderr timing. Fractional seconds are retained without rounding to three decimal places. Process startup and argument parsing are excluded. Errors keep their existing exit status and stderr diagnostics without a JSON error envelope.
- Numeric counts and sizes must be positive integers. Durations must be finite,
  round to at least one nanosecond, and fit a monotonic clock deadline; values
  such as `1e-100` and `1e100` are rejected. Concurrency must be between 1 and
  Tokio's `Semaphore::MAX_PERMITS` (platform-dependent), inclusive.
- With fetching enabled, the default candidate count uses checked `3 * top-k`.
  If that exceeds the platform integer range, lower `--top-k` or explicitly set
  `--fetch-candidates`. No multiplication is required with `--no-fetch` or an
  explicit candidate count. Invalid numeric inputs exit with usage status 2
  before requests, with stderr diagnostics and empty stdout. Defaults and JSON
  schemas are unchanged; replace formerly accepted overflowing values in scripts.
- PDFs are skipped during content fetching.
- Content-quality assessment is advisory: `boilerplate_only` recognizes a limited English whole-message vocabulary; `unflagged` does not certify useful evidence; `unknown` covers missing, mixed, insufficient, or over-limit text. Assessment examines at most 32,768 UTF-8 bytes and describes only retained text, separately from HTTP success and truncation. No quality rejection or ranking penalty is added. Default JSON diagnostics, library methods and opt-in search artifacts expose the assessment; callers must still inspect the text.
- When the selected HTML body consists entirely of recognized shell messages, extraction checks at most the first explicit `article` for non-shell or mixed text. Otherwise root selection is unchanged. This can recover an article hidden by a comments-only `main`; it does not repair arbitrary missing text or render JavaScript. Warm cache entries retain their stored text until expiry; assessments are recomputed, not cached.
- Direct fetch and search HTML/XHTML extraction remove structural chrome and explicit clutter markers using whole class tokens and scoped ID names, not arbitrary substrings. Containers named `download`, `reader`, `shadow`, and `thread` retain their content. Unrecognized compound names may retain clutter; extraction remains heuristic.
- HTML text follows document order, with line breaks between blocks and tabs between table cells. Inline emphasis and links preserve word boundaries; short answers, all heading levels, nested lists, code and table text are retained once per source occurrence. Prose whitespace collapses; `pre` preserves indentation, line breaks and repeated lines. Inline `code` uses prose whitespace rules. Entities are decoded once by HTML parsing; literal metadata lines such as `Source:` remain page content. This is plain text, not Markdown or a rendered table; CSS layout and row/column spans are not reconstructed.
- HTML character limits count Unicode scalar values, including retained whitespace and generated separators, before the CLI source prefix. Truncation can end mid-block or mid-code. HTML headings have no implicit ranking boost: body BM25 uses the extracted tokens once per source occurrence, and existing title/snippet policies keep their ranking weights. New extraction can change scores and which content fits the cap. Old cached extractions retain their prior text until expiry or a fresh fetch; use `--cache-ttl 0` for fresh search page extraction.
- Page bodies stop at `--max-response-bytes` decoded bytes and the retained prefix is extracted, even when Content-Length exceeds the cap. Reaching the cap alone is not an error; content may be incomplete. Network and parsing concurrency are independent.
- Search reports the number of successfully extracted pages that reached the byte cap on stderr; results may contain incomplete page content.
- Byte-capped page extractions are not cached, so a later larger byte budget can fetch more content. This page-fetch cutoff does not change search-provider response limits.
- By default, at most three times `--top-k` candidates are fetched before BM25 ranking.
- BM25 filtering removes zero-relevance results unless an entire query group scores zero.
- Use `--no-fetch` for a fast, low-cost keyword search.
- Provider-native passthrough is the default and only query behavior: shell quotes group one argument without adding local AND or phrase checks. Literal quotes and operators are sent unchanged; provider support varies. Missing title/snippet terms do not reject results. Existing standalone hostname restrictions and HTTP(S) URL validation remain. BM25/ranking handles relevance.
- Portable mode and --query-syntax have been removed. Remove that flag from saved commands; there is no replacement local Boolean/phrase filter. Do not infer full-page relevance from a snippet.
- All nine supported engines are selected by default. Explicit `--engine` selections replace this list; use `-e duckduckgo -e bing -e yahoo` to retain the previous provider set.
- Provider failures, bot challenges, and unsupported region/recency filters retain results from successful providers; including an engine does not guarantee results.
- Fanout defaults to a five-second search budget, including queueing and retries. Use --search-budget to change it or --no-search-budget to disable the total deadline.
- Search, page fetching, and parsing concurrency each default to 10.
- Search/fetch clients select random browser headers and reuse HTTP/2 or HTTP/1.1 connections within the process.
"#;

pub fn generate_skill_md(root: &mut Command) -> String {
    root.build();
    let mut rendered = HEADER.to_owned();
    rendered.push_str(
        r#"## Optional OpenTelemetry / Honeycomb tracing

The CLI exports traces when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (appends
`/v1/traces`); `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` overrides it with a full URL.
Set `OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=$HONEYCOMB_API_KEY"` in the shell
using your secret manager; never put the real key in a prompt or committed file.
Use `https://api.eu1.honeycomb.io` for EU or `https://api.honeycomb.io` for US.
`OTEL_SERVICE_NAME` defaults to `kestrel`. Protocol defaults to `http/protobuf`;
`http/json` is also supported, gRPC is not. Trace-specific protocol/headers/timeout
variables override their general OTLP equivalents. Export timeout defaults to
3000 ms (1–30000); `KESTRELSEARCH_OTEL_SHUTDOWN_MS` defaults to 5000 ms (1–30000).

`KESTRELSEARCH_OTEL_CONTENT=none` is the default. `sanitized` exports bounded
queries, intermediate retrieval snapshots, page text, ranking and final results.
URL credentials/query strings/fragments and OTLP header values are redacted; this
is not a general PII filter. Use sanitized data. `KESTRELSEARCH_OTEL_PAYLOAD_BYTES`
defaults to 8192 (1–65536); `KESTRELSEARCH_OTEL_RESULT_LIMIT` to 20 (1–100).
Truncated payloads are explicitly marked JSON prefixes; omitted results are counted.
Each span has a 64 KiB aggregate content budget; exhaustion is marked explicitly.
`KESTRELSEARCH_OTEL_ENABLED=false` disables the exporter. Without an endpoint
there is no remote export. Invalid telemetry configuration/export failures go to
stderr without changing result JSON or functional exit status. Bounded shutdown
can lose spans on abrupt kill.

`OTEL_TRACES_SAMPLER` defaults to `parentbased_always_on`; also supported:
`always_on`, `always_off`, `traceidratio`, `parentbased_traceidratio` (ratio modes
require finite `OTEL_TRACES_SAMPLER_ARG` in 0–1). Test workflows use `always_on`.
`TRACEPARENT`/`TRACESTATE` carry subprocess parents. Benchmark run IDs retain
`KESTRELSEARCH_BENCHMARK_RUN_ID`; test correlation uses `KESTRELSEARCH_OTEL_RUN_ID`
and `KESTRELSEARCH_OTEL_TEST_ID`. Resource attributes can identify revision/CI runs.

In a source checkout, `python3 scripts/test_traces.py` traces Rust and Python tests;
`--include-ignored` explicitly enables ignored/live cases. Ordinary `cargo test`
is still the parallel correctness check. `scripts/verify_test_traces.py` verifies
local OTLP delivery, not Honeycomb ingestion. See `docs/telemetry.md` for lifecycle,
coverage and the environment-only Honeycomb recipe.

"#,
    );
    let _ = writeln!(
        rendered,
        "## Installation\n\n```bash\ncargo install {}\n```\n",
        env!("CARGO_PKG_NAME")
    );
    rendered.push_str(r#"## Choosing a command

- Use `kestrel search "keywords"` to discover pages about a topic. Search runs DuckDuckGo, Bing, Yahoo, Dogpile, Ecosia, Swisscows, Yep, Qwant, and Mojeek concurrently by default.
- Use `kestrel fetch "https://example.com/path/to/page"` when you already have a page URL and need its contents. Fetch requests that URL directly, without a search provider or BM25 filtering.
- Do not use `search "site:<full-url-to-page>"` as a substitute for fetching a known page. Use `site:example.com keywords` only to discover pages within a site.

## Task workflow: discover, inspect, read, stop

1. **Known URL:** fetch it directly. For a quotation or verification, inspect the
   actual extracted passage and its context before citing it.
2. **Unknown sources:** discover a small metadata candidate set, then inspect
   titles, URLs and snippets for relevance, primary-source authority and freshness.
   Keep alternatives when pages may fail; one returned candidate leaves no backup.
3. **Read selectively:** fetch the most promising URL(s). Snippet matches are
   discovery evidence, not full-page support. Details, verification and quotations
   need page evidence when snippets are insufficient. A successful extraction can
   still be boilerplate, a browser-error page or unrelated text; null content and
   truncated text may also leave the question unsupported.
4. **Recover within a bound:** for an ordinary lookup, start with a ceiling of
   two discovery calls and three direct fetches total, adjusting to the user's
   task/budget. If discovery is empty or weak, use the remaining call to clarify
   terms or increase the collection minimum/search budget. Preserve requested
   phrases and Boolean constraints; explain any proposed relaxation instead of
   silently changing the query intent. Inspect candidate evidence for relevance;
   a snippet may omit important page content. Empty results do not prove that
   no sources exist; inspect `.diagnostics.search` for observed completion conditions.
   If a page fails or is unusable, try an alternative. If evidence was truncated,
   one bounded refetch with larger character/byte limits counts toward the ceiling.
   There is no find-within-page or ranked-passage command today.
5. **Stop:** once the requested claims have adequate source support, stop retrieving.
   If the ceiling is reached first, state what remains uncertain or unsupported;
   do not turn missing text into a negative factual conclusion. Broader research
   can justify a larger explicit budget, not an unbounded retry loop.

### Low-latency discovery

```bash
kestrel search "rust ownership" --no-fetch -k 3 --min-results 3 --search-budget 2 --output json
```

Inspect `.results[]` in the JSON object, choose a source, then fetch its URL.
A one-result/one-second search is an aggressive coverage tradeoff, not a universal
optimum. `--top-k` caps output; `--min-results` controls accepted collection per
query. Neither promises enough useful evidence. The search budget excludes client
initialization, fetching, ranking and output; it is not end-to-end wall time.

### Evidence-seeking discovery and reading

```bash
kestrel search "rust ownership" --no-fetch -k 5 --min-results 10 --search-budget 5 --output json
kestrel fetch "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html" --content-limit 40000 --max-response-bytes 2000000 --timeout 15 --output json
```

The URL illustrates a selected primary source; choose from the actual results.
Inspect fetch `.content` and supporting context, not only exit status. More
alternatives and larger page limits cost work and still do not guarantee support.
For a known-URL task, skip the discovery command entirely.

Connection pools are reused within one process/retained library client. Separate
CLI calls can each incur initialization and cannot reuse the previous process's
pool. Search's opt-in extracted-page cache reuses unexpired completed extractions,
keyed by canonical URL and content limit; provider discovery still runs. Standalone
fetch does not use that cache, and byte-capped extractions are excluded. It is not
cross-process search-progress recovery (tracked in #70).

"#);
    for name in ["search", "fetch"] {
        // Clap selects long help only when that subcommand has long-help content.
        // Follow the actual --help path rather than guessing its render mode.
        let help = root
            .clone()
            .try_get_matches_from([root.get_name(), name, "--help"])
            .err()
            .filter(|error| error.kind() == clap::error::ErrorKind::DisplayHelp)
            .map(|error| error.to_string());
        let Some(command) = root.find_subcommand_mut(name) else {
            continue;
        };
        let _ = writeln!(rendered, "## `{name}` subcommand\n");
        // Let Clap render positionals, placeholders, repeatability, defaults and
        // value choices exactly as it does for the executable's help output.
        let _ = writeln!(
            rendered,
            "```text\n{}\n```",
            help.unwrap_or_else(|| command.render_help().to_string())
        );
        let mut conflicts = std::collections::BTreeSet::new();
        for argument in command.get_arguments() {
            for other in command.get_arg_conflicts_with(argument) {
                let mut pair = [argument.to_string(), other.to_string()];
                pair.sort();
                conflicts.insert(pair);
            }
        }
        if !conflicts.is_empty() {
            rendered.push_str("\nMutually exclusive parameters:\n\n");
            for [first, second] in conflicts {
                let _ = writeln!(rendered, "- `{first}` and `{second}`");
            }
        }
        rendered.push('\n');
    }
    rendered.push_str(
        r#"
## Examples

```bash
kestrel fetch "https://www.rust-lang.org/learn" --output json
kestrel fetch "https://example.com/article" --content-limit 40000 --timeout 20
kestrel fetch "https://example.com/article" --max-response-bytes 65536 --output json
kestrel search "python async patterns" -k 3
kestrel search "rust ownership" --no-fetch --output json
kestrel search "rust ownership" --search-budget 3 --no-fetch
kestrel search "climate news" --time-filter w --region us-en
kestrel search "python typing" -q "pyright docs" -e duckduckgo -e bing
kestrel search "rust async" --no-fetch --ranking-policy snippet --output json
kestrel search "rust async" --ranking-policy hybrid --fetch-budget 5 --fetch-candidates 10
kestrel search "rust async" --cache-ttl 300 --cache-dir .kestrel-cache --cache-max-entries 1000
kestrel search "rust async" --search-concurrency 3 --concurrency 5 --parse-concurrency 2
```
"#,
    );
    rendered.push_str(
        r#"
## Search behavior and parameter requirements

- Quote the primary query; repeat `-q`/`--query` for additional queries. Explicit
  `-e`/`--engine` selections replace the default engine list.
- Fetching and content-only BM25 ranking are enabled by default; `--fetch` and
  `--rank` are optional enable switches, not boolean-valued parameters.
  `--no-fetch` skips page retrieval and default BM25 ranking and conflicts with
  explicit `--rank`. Choose either `--rank` or `--ranking-policy`, never both.
  `--no-rank` skips final ranking but still fetches pages unless `--no-fetch` is set;
  it can be combined with `--pre-rank`, which can change candidate order.
- With `--no-fetch`, omit explicit fetch-stage options: `--fetch-candidates`,
  `--pre-rank`, `--min-fetch-score`, `--content-limit`, `--max-response-bytes`, `--timeout`,
  `--fetch-budget`, `--cache-ttl`, `--cache-dir`, `--cache-max-entries`,
  `--concurrency`, and `--parse-concurrency`. Defaults do not cause conflicts.
  Previously these settings were silently ignored; remove them from search-only
  commands. Conflicting arguments now exit with usage status 2 before requests,
  with an error on stderr and no results on stdout. This also applies to
  `--no-fetch --ranking-policy body` (previously runtime status 1).
- `--pre-rank` scores titles/snippets before selecting fetch candidates, and only
  takes effect when fetching and the candidate count exceeds the fetch limit.
- `--min-fetch-score SCORE` is an opt-in metadata gate, disabled by default.
  It requires page fetching; `--no-fetch` conflicts with it (usage status 2 before requests, empty stdout).
  SCORE must be finite and nonnegative; negative values, NaN and infinities are
  invalid. The comparison is inclusive (`score >= SCORE`), so zero keeps zero
  scores. This is positive-IDF BM25 over doubled title plus snippet, not the
  final content-only `bm25_score`; internal gate scores do not change the JSON
  schema. Scores depend on the query and complete candidate pool, not a fixed
  relevance scale. There is no recommended nonzero cutoff.
  With the gate enabled, search and later stages share trimmed, deduplicated
  queries; whitespace-only queries are dropped and at least one nonempty query
  is required. This keeps score groups aligned with provider provenance.
  Scoring tokenizes the original query text without parsing Boolean operators,
  exclusions or site expressions. Those words can contribute scores, as in other
  lexical ranking. Queries without lexical terms bypass the gate with a stderr
  diagnostic. This replaces the former affirmative-only scoring; site-only and
  exclusion-only queries no longer automatically bypass it. A shared URL survives if any contributing
  query qualifies or bypasses, with all provenance preserved.
  The gate runs before `--pre-rank` and the fetch cap, even for small pools and
  without `--pre-rank`; it removes rejected candidates from fetching AND final
  results. It works with `--no-rank` and every fetch-compatible final policy.
  It does not change provider stopping, refill candidates or relax itself when
  all candidates fail. All-rejected searches perform no page/cache work and
  return a successful empty result. Fewer than top-k results is valid.
  Stderr reports rejected candidates; benchmark diagnostics add `fetch_score`
  timing and `after_fetch_score`, `fetch_score_rejected`, and
  `fetch_score_bypassed_queries` counts. Provider and fetch budgets keep their
  existing scope; metadata scoring is outside both network-stage budgets.
- Experimental `--ranking-policy` choices: `provider` preserves candidate order;
  `snippet` uses titles/snippets; `body` uses content-only BM25 and requires fetching;
  `hybrid` combines title/snippet/body evidence and retains results without bodies;
  `rrf` combines provider ranks. Snippet, hybrid, and RRF also work with `--no-fetch`.
- Search streams normalized results from concurrent providers and stops at five
  unique accepted candidates per query by default. `--min-results N` changes this
  minimum. Provider quorum is ignored, including explicit `--provider-quorum`.
  Duplicate URLs, errors, challenges, and empty responses do not advance the count.
  Query constraints apply before counting. Remaining requests are cancelled and
  their unread results ignored; fusion preserves provenance already received.
  The minimum may be met by one provider; provider diversity is not guaranteed.
- Search defaults to a five-second total deadline and can return fewer results if
  providers finish or the deadline expires. `--no-search-budget` disables this
  deadline but keeps result-count early stopping and individual request timeouts.
- Provider HTML/JSON parsing, including incremental records and completed envelopes,
  uses at most ten queued/running blocking workers per retained client, shared by
  its clones and calls. Cancellation retains capacity until the worker exits.
  Separate clients/free-function calls have separate limits. `--search-concurrency`
  still bounds each call's requests; `--parse-concurrency` controls page extraction,
  not provider workers. Parser queueing is included in the search budget.
- The search budget excludes page fetching. `--fetch-budget` separately bounds the
  candidate-fetch stage and retains completed pages; it is unset by default.
  `--timeout` controls individual page requests, not the total search duration.
- Search page caching is disabled unless `--cache-ttl` is set. Both `--cache-dir`
  and `--cache-max-entries` require `--cache-ttl`. With caching enabled, defaults are
  `~/.cache/kestrel/pages` and 1,000 entries. Standalone `fetch` does not use this cache.
- Search extracts up to 2,000 characters per page by default; standalone `fetch`
  defaults to 20,000. `--content-limit` sets characters; `--max-response-bytes`
  independently caps downloaded bytes.

## Fetch output

`kestrel fetch <URL>` accepts one full HTTP or HTTPS URL. Text output contains
`Source: <url>` followed by the extracted main-body text. `--output json` returns
one object with url, content, elapsed_seconds and default diagnostics. With
`--no-diagnostics`: `{"url": "https://example.com/page", "content": "Source: ...", "elapsed_seconds": 0.125}`.
The default extraction limit is 20,000 characters; increase `--content-limit`
for longer pages. Fetch does not render JavaScript.

Direct fetch and search candidate fetching support `text/plain` as well as
HTML/XHTML. Plain text is decoded using the declared supported charset (UTF-8
when absent or unrecognized) and limited by Unicode characters, preserving line
breaks, indentation, repeated lines, and literal markup/entities such as `<p>`
and `&amp;`. HTML cleanup applies only to HTML/XHTML; responses without a content
type retain the HTML fallback. Invalid byte sequences, including a multibyte
character split by the byte cap, decode with replacement characters. Empty or
whitespace-only retained plain text has no extractable content.
The character cap applies before the CLI adds the `Source:` prefix.

Unsupported content (including PDFs), failed requests, or pages with
no extractable text produce a nonzero exit status and an error on stderr.
`--max-response-bytes` defaults to 1,000,000 decoded body bytes. At the cap,
fetch stops reading without waiting for the rest of the response and extracts
the retained prefix, subject to `--content-limit`. Usable partial content returns
exit status zero with the same text/JSON schema and a notice on stderr. An exact
cap-sized body is conservatively treated as potentially incomplete. If the prefix
contains no extractable text, fetch still fails. Increase the byte cap to retrieve
more of a large page; increasing only `--content-limit` cannot recover unread bytes.
Search page fetching uses the same 1,000,000-byte default and extracts up to
2,000 characters per page. Direct fetch retains its 20,000-character default.
Use `--max-response-bytes 2000000` to restore the previous 2 MB allowance.
Cancellation affects only the current HTTP/2 response stream; HTTP/1.1 is also
supported. In-flight transport bytes may exceed the retained-body cap.
"#,
    );
    rendered.push_str(r#"
## Choosing limits: collected, fetched, returned

These are separate stages, not aliases for one count:

| Option | What it controls | Default and tradeoff |
| --- | --- | --- |
| `--min-results N` | Stop provider collection at N unique accepted candidates **per query** | 5; a larger threshold gives later results a chance but can take longer. Deadlines or exhausted providers may leave fewer. |
| `--fetch-candidates N` | Maximum candidates selected for page fetching across the merged queries | 3 × top-k; does not request more provider results. More candidates can supply alternatives when pages fail or rank poorly, at greater fetch/parse cost. |
| `--min-fetch-score SCORE` | Inclusive metadata BM25 gate before the fetch cap | Disabled; finite nonnegative, tokenized query text. Rejected candidates also leave final output; no universal cutoff. |
| `-k N`, `--top-k N` | Same option: maximum final results across all queries | 5; not a guaranteed result count, collection threshold, or fetch count. |
| `--content-limit CHARS` | Maximum extracted body characters **per page**, before body ranking | Search: 2,000; standalone fetch: 20,000. Shorter text reduces output and ranking input but can omit relevant passages. Not a token limit or total-output cap. |
| `--max-response-bytes BYTES` | Maximum retained decoded response bytes **per page** | 1,000,000; reaching the cap extracts the prefix. A smaller cap reduces retained/downloaded body work but may cut off the article entirely. |

For one query, increasing top-k or fetch-candidates above five does not raise the
default five-candidate collection threshold. Increase `--min-results` as well
when you want a larger pool. With multiple queries, collection is per query,
then URLs are deduplicated and the fetch and final-output ceilings apply globally.
A higher threshold does not guarantee provider diversity or semantic relevance.

A log such as “Got 8 results; Fetching 5 pages; Successfully fetched 4/5;
Returning top 4” describes successive stages, not conflicting options. The
number collected depends on the installed version and query count; older builds
could collect eight for one query before current result-count stopping. The five
selected candidates are not replenished from the unselected results after a
failure. Default body BM25 removes nonpositive scores when a query group has
positive scores; if the entire group scores zero, it retains the group. Thus a
fetch failure can reduce the final count, but successful-fetch count and returned
count are not always equal. `-k 5` promises at most five, never exactly five.

Character limits exclude titles, snippets, URLs, the added `Source:` prefix,
formatting and JSON overhead. Five returned bodies limited to 1,000 characters
contribute at most 5,000 body characters, not 5,000 total output characters.
Reducing `--content-limit` does not itself stop the network download sooner;
use the byte cap for that. Increasing the character limit cannot recover bytes
already cut off by the byte cap. Longer prefixes can expose more useful evidence
but can also contain more irrelevant material; BM25 is lexical, not a guarantee
of semantic quality.

## Optional fetch relevance gate

```bash
kestrel search "rust async" --min-results 15 --fetch-candidates 8 --min-fetch-score 0.1 --no-rank
```

The `0.1` above illustrates syntax, not a calibrated recommendation. Validate
against sources needed for your task; increasing the threshold may lose useful
pages whose metadata provides weak evidence. Omit the flag to preserve current
selection. Regenerate installed skills with the updated binary to learn this
new option.

## Recipes: speed, coverage and relevance

These describe work and coverage tradeoffs, not measured speedups or a quality
ranking. Provider latency, available candidates, cache state and page structure
can change the outcome. The recipes in this section return up to five results.

### Least page work: metadata only

```bash
kestrel search '"machine learning"' -k 5 --no-fetch --output json
kestrel search '"machine learning"' -k 5 --no-fetch --ranking-policy snippet --output json
```

Both skip page downloads and extraction, generally the largest saving. The first
keeps merged provider order; the second adds inexpensive title/snippet ranking.
Neither evaluates page-body evidence. Do not add fetch-stage limits to no-fetch
commands. Narrowing `--engine` reduces provider requests but can lose coverage.

### Bounded page work and short output

```bash
kestrel search '"machine learning"' -k 5 --min-results 5 --fetch-candidates 5 --content-limit 1000 --search-budget 3 --fetch-budget 2 --timeout 2
```

Collect up to the per-query threshold, select at most five candidates, and rank
short extracted bodies. The search budget bounds provider work, the fetch budget
bounds the entire page-fetch stage, and timeout bounds each page request. Tight
budgets cancel slow work and may leave fewer useful results. They are separate
stage limits, not an exact end-to-end deadline including initialization/output.
This favors less work and shorter output over ranking breadth or complete pages.

### More evidence and alternatives for body ranking

```bash
kestrel search '"machine learning"' -k 5 --min-results 15 --fetch-candidates 15 --content-limit 5000 --search-budget 10 --fetch-budget 10
```

Seek fifteen candidates, fetch at most fifteen, then choose up to five using
body BM25. Compared with the bounded recipe, this allows more collection time,
more page work and longer passages. It gives ranking more opportunities to find
relevant evidence and tolerate failures, but may be slower and is not guaranteed
to return five or improve relevance. Raising only fetch-candidates would not
raise the collection threshold. The default 1 MB byte cap still applies.

### Broader discovery with fewer page fetches

```bash
kestrel search '"machine learning"' -k 5 --min-results 20 --fetch-candidates 8 --pre-rank --content-limit 3000 --search-budget 10 --fetch-budget 5
```

Pre-rank titles/snippets from the collected pool, then fetch at most eight pages.
This reduces page requests relative to fetching all twenty, while still allowing
body ranking. Snippets can miss valuable pages, and collecting twenty may itself
be slow. Pre-ranking has no effect if the pool is no larger than the fetch limit.

### Keep metadata candidates when page fetching fails

```bash
kestrel search '"machine learning"' -k 5 --min-results 15 --fetch-candidates 15 --ranking-policy hybrid --content-limit 3000 --fetch-budget 5
```

Hybrid uses title, snippet and available body evidence and retains candidates
without bodies. It may return five items even when fewer than five pages were
read; inspect `content` before treating a result as fetched evidence. It neither
refills fetch slots nor guarantees five results. `--no-rank` also skips body-score
filtering, but keeps candidate order instead of selecting by body relevance and
still fetches pages. Use `--no-fetch` when page text is not needed.

### Repeated searches: reuse extracted pages

```bash
kestrel search '"machine learning"' -k 5 --min-results 15 --fetch-candidates 15 --content-limit 3000 --cache-ttl 300 --cache-max-entries 1000
```

Run again with the same settings to reuse unexpired page entries for up to five
minutes. The first run still pays cold-fetch cost; provider search still runs on
every invocation. Cache hits can save page requests, but text may be stale within
the TTL. Changing the content limit changes the cache key and can miss the cache.
More concurrency can overlap waits but raises resource use and is not always
faster; lower concurrency reduces simultaneous work and may increase latency.

### Known URL: read more of one page directly

```bash
kestrel fetch "https://example.com/article" --content-limit 40000 --max-response-bytes 2000000 --timeout 20 --output json
```

This bypasses discovery and ranking. It allows more body bytes and extracted
characters than the defaults, potentially taking longer. Extraction can still
return less text; JavaScript is not rendered and truncated prefixes may lack the
article. Search and direct-fetch content limits are per page, not total output.

"#);
    rendered.push_str(SCHEMA_AND_NOTES);
    rendered.push_str(
        r#"
## Binary installation and removal of skills

`kestrel install` copies the running executable on macOS/Linux; it does not
fetch a release or update the binary. Defaults to `~/.local/bin/kestrel`.
Use `--system` for `/usr/local/bin/kestrel` (usually requires elevated privileges),
or `--dir DIRECTORY` for a custom destination; these flags are mutually exclusive.

```bash
kestrel install --dir ./bin
kestrel skill uninstall
```

Self-install creates the directory and prints PATH guidance without editing shell
configuration. Ensure the destination is on PATH and check for an earlier binary.
Copying the installed file onto itself is a no-op. Other existing files/symlinks
prompt `Replace it? [y/N]`; only y/yes replaces them, leaving a symlink's target
untouched. Use the package manager to update a package-managed installation.
Prebuilt releases currently support Apple Silicon macOS; Cargo can build on Linux.

`skill uninstall` interactively selects recorded installations from
`~/.kestrelsearch/config.toml`, removes selected skill files and cleans stale
records. Its selection prompt defaults to all: choose numbered paths to remove
only intended copies. It does not discover unrecorded copies or remove the binary,
and has no agent/scope/force switches. Restart the agent after removal.

Installation records use a persistent `config.toml.lock` beside the config
(or its resolved symlink destination). Do not delete this lock file. Cooperating
Kestrel processes serialize config updates; lock contention fails after ten seconds
with an error on stderr and a nonzero exit status. Retry after the other installation
finishes. Config writes use atomic replacement and preserve unrelated TOML and
existing file permissions. Invalid TOML returns an error without overwriting it;
repair or restore the config before retrying. Existing config symlinks are followed;
dangling symlinks are rejected.

Staged contents are synced before replacement; on Unix the directory is synced
afterwards. Errors before replacement preserve the previous config. A directory-sync
error can occur after the new config is already visible. This does not guarantee
power-loss durability on every filesystem. Older binaries and external editors
that ignore the lock are not protected. Skill file creation/removal and config
bookkeeping are separate operations: an error may leave a skill file unrecorded
or a stale record. Inspect the intended paths before retrying.

## Advanced diagnostics (optional)

Use these environment interfaces to investigate a specific failure, not for every
lookup. In a shell, create private temporary directories and enable capture for
one command:

```bash
trace_dir=$(mktemp -d)
artifact_dir=$(mktemp -d)
KESTRELSEARCH_PROVIDER_TRACE_DIR="$trace_dir" KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR="$artifact_dir" KESTRELSEARCH_BENCHMARK_RUN_ID=lookup kestrel search "rust ownership" --no-fetch --output json
```

- `KESTRELSEARCH_PROVIDER_TRACE_DIR` writes generated-header files, provider
  `outcome-*.json` lifecycle records and available raw response `.html`/metadata
  captures. Interrupted or oversized bodies may have no raw capture. Correlate
  run/search/attempt IDs; raw files and outcomes are views of the same attempts.
  Cancelled work may already have supplied retained candidates. A censored phase
  is time observed before cancellation, not a completed latency measurement.
- Search artifact writing requires **both** `KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR`
  and `KESTRELSEARCH_BENCHMARK_RUN_ID`. Use a simple filename label such as `lookup`.
  Successful search handling writes `<run-id>-<uuid>.json` in the chosen directory,
  with candidate snapshots, phase timings and provider/fetch reports. This is a
  separate diagnostic schema, not the ordinary stdout JSON envelope. Each returned
  artifact result includes `content_quality` (`version`, `state`, `reasons`);
  `diagnostics.candidate_content_quality` aligns with the artifact's `candidates`
  array, not result ranks or fetch-page diagnostics. Version 1 is an advisory
  whole-message check, not an evidence score. Standalone
  fetch does not write these search artifacts. Failed searches can still produce
  provider lifecycle traces without a search artifact.
- Traces/artifacts may contain queries, URLs and raw responses or extracted content;
  they are not anonymized. Inspect and redact sensitive data before sharing. Trace
  metadata omits response cookies/authorization headers, but bodies may be sensitive.
  Capture adds I/O overhead; do not treat instrumented timings as free of that cost.
- Independently, best-effort local event logging writes selected search/fetch
  events to `~/.kestrel/logs/YYYY-MM-DD/events.jsonl` (UTC date). This is an existing
  side effect, not opt-in provider tracing, a complete report or a reliable resume
  journal. There are no CLI disable/location controls. Logs can include query/URL
  information; missing events do not prove that work did not happen.

## Default structured diagnostics and migration

Both JSON commands add `diagnostics` by default. This is an intentional default
schema extension; `--no-diagnostics` restores the previous JSON object. The flag
has no effect in text mode. No persistence or trace directory is required.
Search diagnostics use `schema_version: 1` and contain:

- `search`: normalized query_count, minimum_per_query, per-query `queries` with
  query_index, unique_accepted, minimum_reached, deadline, providers_exhausted,
  all_failed; aggregate counts minimum_reached_queries, deadline_queries,
  providers_exhausted_queries, all_failed_queries, all_minimum_reached and
  budget_exhausted. Conditions are independent and can overlap. Exhaustion
  includes finished errors, but excludes deadline/cancellation. Successful empty
  or all-filtered responses are not all_failed. Query indices follow trimmed,
  nonempty, deduplicated inputs. Shared URLs count once per contributing query.
- `search.provider_outcomes`: counts of all scheduled provider/query outcomes,
  including omitted detail rows; absent keys mean zero.
- `search.providers`: engine, query_index, outcome, response_completed_successfully,
  raw, rejected, accepted_snapshot, retained_occurrences, retries, elapsed_ms,
  timing_censored. Cancelled providers can contribute retained candidates.
  Raw equals rejected plus accepted_snapshot in final cumulative observations;
  never sum successive snapshots. Aggregate retained_occurrences counts source
  occurrences, while unique_accepted counts fused URLs. Failed responses can
  retract snapshots; rejected_response_snapshot counts those observations.
  Snapshot publication can race with cancellation; use retained_occurrences to
  identify contribution. timing_censored is true for deadline/cancellation,
  false for completed success and null for unavailable error-phase semantics.
- `candidates`: unique_accepted, fetch_score_rejected, after_fetch_score,
  after_selection, not_selected, after_ranking, returned. The first minus the
  score rejections equals after_fetch_score; not_selected includes score/cap
  removals. Ranking and top-k can further reduce output without refilling.
- `evidence`: enabled, selected, scheduled (excludes existing .pdf URL skips),
  completed (includes errors/cache hits), extracted, usable (null),
  usefulness_unknown (all extracted bodies), quality counts, states,
  budget_exhausted, cancelled, cache_hits. Counts precede ranking/top-k;
  absent count-map keys mean zero. Quality covers the post-selection pool,
  including missing bodies, and does not certify useful evidence.
- `pages`: candidate_index in the original unique pool, returned_index (null
  when not returned), state, quality, timing, byte_cap_reached. States distinguish
  no_fetch, not_selected, skipped_pdf, extracted, cache_hit, empty_extraction,
  unsupported_content_type, response_too_large, request_failed, fetch_deadline
  and unknown. A missing page timing is null; request-failure censoring is unknown.
  Use `.results[returned_index].url` for selective reading of returned pages.

Detail lists stop at 128 queries, 128 providers and 256 pages; corresponding
queries_omitted/providers_omitted (inside search) and pages_omitted disclose
truncation. Aggregates remain complete. Diagnostics omit raw query/URL strings,
page bodies, cookies, credentials and verbose transport messages. Existing
result fields retain their content. Hybrid score components remain unavailable.

Standalone fetch diagnostics have schema_version, state, quality, usable (null),
timing, byte_cap_reached and budget_exhausted; no search, cache or batch controls.
Both commands retain existing error behavior: invalid arguments exit 2, runtime
failure exits 1, stderr carries errors, stdout has no JSON error envelope.
All-failed search and standalone fetch with no text therefore have no JSON
report; successful empty search exits 0 with diagnostics. Failed queries within
a successful multi-query search have all_failed=true.

For retry decisions inspect `.diagnostics.search.budget_exhausted` alongside
`.diagnostics.search.all_minimum_reached`; an observed deadline does not mean
all results failed. Inspect snippets, then fetch a selected result URL if its
page state is no_fetch or evidence is insufficient. A larger budget is only a
retry choice, not a guarantee of more evidence. Do not equate unflagged or
successful extraction with usefulness.

## Refreshing an installed skill

Locate the intended Rust executable with `command -v kestrel` (and `type -a kestrel`
where supported), then check that exact path with `--version`, `search --help` and
`fetch --help`. The Cargo package is `kestrel-rs`; the binary is `kestrel`, distinct
from the Python `kestrelsearch` executable. When PATH is ambiguous, invoke the
verified absolute binary path for installation and refresh.

Inspect the intended project/global skill paths before refreshing stale copies.
Project paths are `.claude/skills`, `.codex/skills`, and `.github/skills`; global
paths are `~/.claude/skills`, `~/.codex/skills`, and `~/.copilot/skills`, each with
`kestrelsearch/SKILL.md`. Refresh only the agent/scope you intend, not unrelated
installations or every copy merely because one is stale.

After updating the executable, regenerate the skill with that executable. For
example, overwrite the current user's Codex installation with:

```bash
kestrel skill install --agent codex --scope global --force
```

Use `--scope project` from the target project for a project installation.
Agents are `claude`, `codex`, `vscode`, or `all`; `both` means Claude and VS Code.
Omitting agent or scope prompts interactively. `--force` overwrites an existing
skill without prompting. Restart the agent session after regenerating the file.
Check `kestrel --version` and `kestrel search --help` if installed behavior differs
from this reference; the generated reference reflects the executable that wrote it.
"#,
    );
    rendered
}
