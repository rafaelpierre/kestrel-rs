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

"#;

const SCHEMA_AND_NOTES: &str = r#"
## Search JSON output schema

`search --output json` returns an object with `results` (an array, empty when no
results are found) and `elapsed_seconds` (a finite, nonnegative number in seconds).
Example: `{"results": [], "elapsed_seconds": 0.125}`.
This is a breaking change from the previous top-level array. Read `.results`
instead of the root array (for example, migrate `jq '.[]'` to `jq '.results[]'`).
There is no legacy-output flag; the library result types are unchanged.
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
- Numeric counts and sizes must be positive integers; durations must be finite and greater than zero.
- PDFs are skipped during content fetching.
- Page bodies stop at `--max-response-bytes` decoded bytes and the retained prefix is extracted, even when Content-Length exceeds the cap. Reaching the cap alone is not an error; content may be incomplete. Network and parsing concurrency are independent.
- Search reports the number of successfully extracted pages that reached the byte cap on stderr; results may contain incomplete page content.
- Byte-capped page extractions are not cached, so a later larger byte budget can fetch more content. This page-fetch cutoff does not change search-provider response limits.
- By default, at most three times `--top-k` candidates are fetched before BM25 ranking.
- BM25 filtering removes zero-relevance results unless an entire query group scores zero.
- Use `--no-fetch` for a fast, low-cost keyword search.
- Portable query syntax is the default for every provider: quoted phrases require adjacency in one title/snippet; unquoted terms use AND. Uppercase AND/OR/NOT, exclusions, parentheses and site:hostname are supported.
- Query constraints filter title/snippet evidence before counting toward the result minimum, independently of fetching/ranking. Missing positive evidence excludes a result; NOT checks metadata absence, not the full page.
- Use --query-syntax native for provider-specific syntax such as filetype:pdf and the previous passthrough behavior. Do not claim complete-page relevance from metadata matches.
- All nine supported engines are selected by default. Explicit `--engine` selections replace this list; use `-e duckduckgo -e bing -e yahoo` to retain the previous provider set.
- Provider failures, bot challenges, and unsupported region/recency filters retain results from successful providers; including an engine does not guarantee results.
- Fanout defaults to a five-second search budget, including queueing and retries. Use --search-budget to change it or --no-search-budget to disable the total deadline.
- Search, page fetching, and parsing concurrency each default to 10.
- Search/fetch clients select random browser headers and reuse HTTP/2 or HTTP/1.1 connections within the process.
"#;

pub fn generate_skill_md(root: &mut Command) -> String {
    root.build();
    let mut rendered = HEADER.to_owned();
    let _ = writeln!(
        rendered,
        "## Installation\n\n```bash\ncargo install {}\n```\n",
        env!("CARGO_PKG_NAME")
    );
    rendered.push_str(r#"## Choosing a command

- Use `kestrel search "keywords"` to discover pages about a topic. Search runs DuckDuckGo, Bing, Yahoo, Dogpile, Ecosia, Swisscows, Yep, Qwant, and Mojeek concurrently by default.
- Use `kestrel fetch "https://example.com/path/to/page"` when you already have a page URL and need its contents. Fetch requests that URL directly, without a search provider or BM25 filtering.
- Do not use `search "site:<full-url-to-page>"` as a substitute for fetching a known page. Use `site:example.com keywords` only to discover pages within a site.

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
  `--pre-rank`, `--content-limit`, `--max-response-bytes`, `--timeout`,
  `--fetch-budget`, `--cache-ttl`, `--cache-dir`, `--cache-max-entries`,
  `--concurrency`, and `--parse-concurrency`. Defaults do not cause conflicts.
  Previously these settings were silently ignored; remove them from search-only
  commands. Conflicting arguments now exit with usage status 2 before requests,
  with an error on stderr and no results on stdout. This also applies to
  `--no-fetch --ranking-policy body` (previously runtime status 1).
- `--pre-rank` scores titles/snippets before selecting fetch candidates, and only
  takes effect when fetching and the candidate count exceeds the fetch limit.
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
one object: `{"url": "https://example.com/page", "content": "Source: ...", "elapsed_seconds": 0.125}`.
The default extraction limit is 20,000 characters; increase `--content-limit`
for longer pages. Fetch uses the existing HTML/text extractor and does not render
JavaScript. Unsupported content (including PDFs), failed requests, or pages with
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

## Recipes: speed, coverage and relevance

These describe work and coverage tradeoffs, not measured speedups or a quality
ranking. Provider latency, available candidates, cache state and page structure
can change the outcome. All examples return up to five results.

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
## Refreshing an installed skill

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
