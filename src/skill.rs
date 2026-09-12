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

`--output json` returns an array (empty when no results are found). Each result has
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
- Numeric counts and sizes must be positive integers; durations must be finite and greater than zero.
- PDFs are skipped during content fetching.
- Page bodies are streamed up to `--max-response-bytes`; network and parsing concurrency are independent.
- By default, at most three times `--top-k` candidates are fetched before BM25 ranking.
- BM25 filtering removes zero-relevance results unless an entire query group scores zero.
- Use `--no-fetch` for a fast, low-cost keyword search.
- Additional opt-in engines: dogpile, ecosia, swisscows, yep, qwant, mojeek.
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

- Use `kestrel search "keywords"` to discover pages about a topic. Search runs DuckDuckGo, Bing, and Yahoo concurrently by default.
- Use `kestrel fetch "https://example.com/path/to/page"` when you already have a page URL and need its contents. Fetch requests that URL directly, without a search provider or BM25 filtering.
- Do not use `search "site:<full-url-to-page>"` as a substitute for fetching a known page. Use `site:example.com keywords` only to discover pages within a site.

"#);
    for name in ["search", "fetch"] {
        let Some(command) = root.find_subcommand_mut(name) else {
            continue;
        };
        let _ = writeln!(rendered, "## `{name}` subcommand\n");
        // Let Clap render positionals, placeholders, repeatability, defaults and
        // value choices exactly as it does for the executable's help output.
        let _ = writeln!(rendered, "```text\n{}\n```", command.render_help());
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
  `--no-fetch` skips page retrieval and default BM25 ranking, even with `--rank`.
  `--no-rank` skips final ranking but still fetches pages unless `--no-fetch` is set.
- `--pre-rank` scores titles/snippets before selecting fetch candidates, and only
  takes effect when fetching and the candidate count exceeds the fetch limit.
- Experimental `--ranking-policy` choices: `provider` preserves candidate order;
  `snippet` uses titles/snippets; `body` uses content-only BM25 and requires fetching;
  `hybrid` combines title/snippet/body evidence and retains results without bodies;
  `rrf` combines provider ranks. Snippet, hybrid, and RRF also work with `--no-fetch`.
- Search defaults to a five-second total deadline with no provider quorum.
  Explicit `--search-budget` without `--mode` selects quorum 1; explicit
  `--provider-quorum` overrides it. `--mode fanout` suppresses that implicit quorum.
  Quorum counts nonempty provider responses per query, not relevance or result count.
  `--no-search-budget` disables the total search deadline; request timeouts remain.
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
one object: `{"url": "https://example.com/page", "content": "Source: ..."}`.
The default extraction limit is 20,000 characters; increase `--content-limit`
for longer pages. Fetch uses the existing HTML/text extractor and does not render
JavaScript. Unsupported content (including PDFs), failed requests, or pages with
no extractable text produce a nonzero exit status and an error on stderr.
"#,
    );
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
