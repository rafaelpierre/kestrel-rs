//! Agent skill content generated from the live Clap command tree.

use std::fmt::Write;

use clap::Command;

const HEADER: &str = r#"---
name: kestrelsearch
description: >
  Lightweight multi-engine web search with BM25 relevance ranking.
  Use when asked to search the web, look something up, find recent information,
  research a topic, or browse the internet.
  Trigger phrases: search the web, look up, find information about, google,
  browse the web, web search, find recent, what is the latest on.
argument-hint: "<search query>"
---

# Kestrel Search

Kestrel Search — web search, page extraction, and relevance ranking for AI agents.

## Installation

```bash
cargo install kestrelsearch
```

"#;

const SCHEMA_AND_NOTES: &str = r#"
## JSON output schema

Each element in the returned array contains:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Page title |
| `url` | string | Full canonical URL |
| `display_url` | string | Shortened URL shown by the search engine |
| `snippet` | string | Search-result snippet |
| `content` | string or null | Extracted main-body text prefixed with `Source: <url>` |
| `bm25_score` | number | BM25 relevance score when ranking is active |
| `engine` | string | Engine that supplied the retained result |
| `query` | string | Query that supplied the retained result |
| `engine_rank` | number | Original provider/query position |
| `sources` | array | Every provider/query occurrence merged into this URL |

## Notes

- This `SKILL.md` is compatible with Claude Code, Codex, and GitHub Copilot in VS Code.
- Progress logs go to **stderr**; clean JSON goes to **stdout**.
- PDFs are skipped during content fetching.
- Page bodies are streamed up to `--max-response-bytes`; network and parsing concurrency are independent.
- By default, at most three times `--top-k` candidates are fetched before BM25 ranking.
- BM25 filtering removes zero-relevance results unless an entire query group scores zero.
- Use `--no-fetch` for a fast, low-cost keyword search.
"#;

pub fn generate_skill_md(root: &mut Command) -> String {
    root.build();
    let mut rendered = HEADER.to_owned();
    let Some(search) = root.find_subcommand("search") else {
        rendered.push_str(SCHEMA_AND_NOTES);
        return rendered;
    };
    rendered.push_str("## `search` subcommand\n\n");
    rendered.push_str(
        search
            .get_about()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "Search one or more engines and return ranked results.".into())
            .as_str(),
    );
    rendered.push_str("\n\n### Options\n\n| Option | Default | Choices | Description |\n|--------|---------|---------|-------------|\n");
    for argument in search
        .get_arguments()
        .filter(|argument| argument.get_long().is_some())
    {
        let mut names = String::new();
        if let Some(short) = argument.get_short() {
            let _ = write!(names, "-{short}/");
        }
        if let Some(long) = argument.get_long() {
            let _ = write!(names, "--{long}");
        }
        let defaults = argument
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        let choices = argument
            .get_value_parser()
            .possible_values()
            .map(|values| {
                values
                    .map(|value| value.get_name().to_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let help = argument
            .get_help()
            .map(|value| value.to_string().replace('|', "\\|"))
            .unwrap_or_default();
        let _ = writeln!(rendered, "| `{names}` | {defaults} | {choices} | {help} |");
    }
    rendered.push_str(
        r#"
### Examples

```bash
kestrel search "python async patterns" -k 3
kestrel search "rust ownership" --no-fetch --output json
kestrel search "climate news" --time-filter w --region us-en
kestrel search "python typing" -q "pyright docs" -e duckduckgo -e bing --mode fanout
```
"#,
    );
    rendered.push_str(SCHEMA_AND_NOTES);
    rendered
}
