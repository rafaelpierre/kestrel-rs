# Search and fetch argument responsibilities

Search has three stages: provider search, optional candidate/page fetching, and
optional final ranking. Standalone `fetch` reads one URL and has no provider or
ranking stage. Flags shared by the commands retain the same units and purpose;
search defaults to 2,000 extracted characters per page and fetch to 20,000.

| Controls | Responsibility and interaction |
| --- | --- |
| Query, `--query`, `--query-syntax`, `--engine`, `--region`, `--time-filter` | Provider queries and filters; unaffected by fetching or ranking switches. |
| `--search-concurrency` | Concurrent provider requests. |
| `--search-budget`, `--no-search-budget` | Mutually exclusive total provider deadline controls; excludes page fetching. |
| `--mode fanout`, `--provider-quorum` | Fanout is the only mode; provider quorum is ignored by result-count stopping. |
| `--min-results` | Provider stopping threshold per query (default five unique accepted candidates); independent of fetch and return limits, and not a guarantee. |
| `--top-k`, `--fetch-candidates` | Returned result ceiling versus page candidate ceiling (default three times top-k). A smaller candidate ceiling can intentionally return fewer than top-k results. |
| `--pre-rank` | Orders titles/snippets before limiting fetch candidates, only when candidates exceed the limit. Independent of final ranking. |
| `--fetch`, `--no-fetch` | Enable switch (already the default) versus disabling page retrieval and default body ranking. Mutually exclusive. |
| `--rank`, `--no-rank`, `--ranking-policy` | Choose at most one explicit final-ranking control. Default is content BM25 with fetching. Provider policy preserves candidate order, like no-rank. |
| `--timeout`, `--fetch-budget` | Individual page-request timeout versus total candidate-fetch budget. Both may apply; standalone fetch only needs the request timeout. Neither sets provider-request timeouts. |
| `--content-limit`, `--max-response-bytes` | Extracted character ceiling versus downloaded byte ceiling. Neither implies the other. |
| `--concurrency`, `--parse-concurrency` | Concurrent page requests versus HTML parsing jobs. Standalone fetch reads one URL, so exposes neither. |
| `--cache-ttl`, `--cache-dir`, `--cache-max-entries` | Search page-cache lifetime, location, and capacity. Directory/capacity require TTL; standalone fetch does not use the cache. |
| `--output` | Text or JSON, independently available for both commands. Search JSON contains results and elapsed_seconds; fetch JSON contains url, content and elapsed_seconds. |

## Invalid combinations and migration

`--no-fetch` conflicts with explicit `--fetch`, `--rank`, `--fetch-candidates`,
`--pre-rank`, `--content-limit`, `--max-response-bytes`, `--timeout`, `--fetch-budget`,
`--cache-ttl`, `--cache-dir`, `--cache-max-entries`, `--concurrency`, and
`--parse-concurrency`. Defaults do not cause conflicts; only explicitly supplied
options are checked. Remove these settings from search-only commands, or remove
`--no-fetch` to enable their stage.

`--rank` conflicts with every explicit `--ranking-policy`, even `body`: choose
one control instead of supplying redundant or contradictory ranking instructions.
`--no-rank` continues to conflict with `--rank` and every explicit policy.

`--no-fetch --ranking-policy body` is invalid because body BM25 needs fetching.
Provider, snippet, hybrid and RRF policies remain valid without fetching.
`--no-fetch --no-rank` also remains valid. `--pre-rank --no-rank` is valid with
fetching: it skips final ranking but can still reorder the candidate list.

Conflicting combinations now fail before any requests, with exit status 2, an
error on stderr and empty stdout. Previously fetch-stage controls with no-fetch
and overridden rank switches were accepted but silently ignored; body/no-fetch
failed with runtime status 1. This is an intentional CLI compatibility change.
Valid invocations, defaults, flag names, library behavior and output schemas are
unchanged. Regenerate installed skills with the updated binary.

For example, replace `search "rust" --no-fetch --timeout 10 --rank` with
`search "rust" --no-fetch`, or use `search "rust" --no-fetch --ranking-policy snippet`
to explicitly rank metadata. Replace `search "rust" --rank --ranking-policy hybrid`
with `search "rust" --ranking-policy hybrid`.

The numeric overflow work tracked in issue #24 is separate from this argument
interaction audit. Engine defaults and fetch response-limit changes are likewise
tracked separately in #62 and #53.
