# Search and fetch argument responsibilities

Search has three stages: provider search, optional candidate/page fetching, and
optional final ranking. Standalone `fetch` reads one URL and has no provider or
ranking stage. Flags shared by the commands retain the same units and purpose;
search defaults to 2,000 extracted characters per page and fetch to 20,000.

| Controls | Responsibility and interaction |
| --- | --- |
| Query, `--query`, `--engine`, `--region`, `--time-filter` | Provider queries and filters; unaffected by fetching or ranking switches. |
| `--search-concurrency` | Concurrent provider requests. |
| `--search-budget`, `--no-search-budget` | Mutually exclusive total provider deadline controls; excludes page fetching. |
| `--mode fanout`, `--provider-quorum` | Fanout is the only mode; provider quorum is ignored by result-count stopping. |
| `--min-results` | Provider stopping threshold per query (default five unique accepted candidates); independent of fetch and return limits, and not a guarantee. |
| `--top-k`, `--fetch-candidates` | Returned result ceiling versus page candidate ceiling (default three times top-k). A smaller candidate ceiling can intentionally return fewer than top-k results. |
| `--min-fetch-score` | Optional inclusive positive-IDF metadata threshold before candidate truncation; fetching required; independent of final ranking. |
| `--pre-rank` | Orders titles/snippets before limiting fetch candidates, only when candidates exceed the limit. Independent of final ranking. |
| `--fetch`, `--no-fetch` | Enable switch (already the default) versus disabling page retrieval and default body ranking. Mutually exclusive. |
| `--rank`, `--no-rank`, `--ranking-policy` | Choose at most one explicit final-ranking control. Default is content BM25 with fetching. Provider policy preserves candidate order, like no-rank. |
| `--timeout`, `--fetch-budget` | Individual page-request timeout versus total candidate-fetch budget. Both may apply; standalone fetch only needs the request timeout. Neither sets provider-request timeouts. |
| `--content-limit`, `--max-response-bytes` | Extracted character ceiling versus downloaded byte ceiling. Neither implies the other. |
| `--concurrency`, `--parse-concurrency` | Concurrent page requests versus HTML parsing jobs. Standalone fetch reads one URL, so exposes neither. |
| `--cache-ttl`, `--cache-dir`, `--cache-max-entries` | Search page-cache lifetime, location, and capacity. Directory/capacity require TTL; standalone fetch does not use the cache. |
| `--output` | Text or JSON, independently available for both commands. Search JSON contains results and elapsed_seconds; fetch JSON contains url, content and elapsed_seconds. |

Both JSON commands include default versioned diagnostics. `--no-diagnostics`
restores their previous envelopes and has no effect on text output. See the
[contract, bounds and migration](structured-diagnostics.md).

## HTML text layout

HTML/XHTML content follows source order with line breaks between blocks, tabs
between table cells and preserved `pre` formatting. Short answers and all heading
levels are retained. Character limits include whitespace and separators; schemas
and CLI flags are unchanged. See [ordered extraction and ranking implications](page-extraction.md#ordered-readable-text-issue-22).

## Plain-text responses

Direct fetch and search candidate fetching support `text/plain` as well as
HTML/XHTML. Plain text is decoded using the declared supported charset (UTF-8
when absent or unrecognized) and limited by Unicode characters, preserving line
breaks, indentation, repeated lines, and literal markup/entities such as `<p>`
and `&amp;`. HTML cleanup applies only to HTML/XHTML; responses without a content
type retain the HTML fallback. Invalid byte sequences, including a multibyte
character split by the byte cap, decode with replacement characters. Empty or
whitespace-only retained plain text has no extractable content.

The character cap applies to the decoded body before the CLI adds its `Source:`
prefix. Byte limits, timeouts, and partial-response diagnostics apply as usual.

## Invalid combinations and migration

`--no-fetch` conflicts with explicit `--fetch`, `--rank`, `--fetch-candidates`,
`--pre-rank`, `--min-fetch-score`, `--content-limit`, `--max-response-bytes`, `--timeout`, `--fetch-budget`,
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

## Numeric boundaries

Counts and sizes must be positive integers representable by the platform's
`usize`. Provider, page and parser concurrency additionally cannot exceed
Tokio's `Semaphore::MAX_PERMITS` (inclusive, platform-dependent).

All CLI seconds values must convert to a nonzero `Duration` and fit a monotonic
clock deadline. This rejects nonfinite values, zero, values rounding below one
nanosecond (such as `1e-100`), and overflowing values (such as `1e100`).
With fetching enabled and no explicit `--fetch-candidates`, `3 * top-k` must fit
`usize`. Lower `--top-k` or set `--fetch-candidates` explicitly; metadata-only
searches do not calculate that default.

Invalid numeric arguments fail before requests with usage status 2, stderr
explanations and empty stdout. Previously some values panicked or wrapped.
Ordinary defaults and JSON schemas are unchanged. Regenerate installed skills
and replace overflowing values in scripts.

Library search/fetch options reject out-of-range concurrency and zero or
unrepresentable timeout/budget deadlines with `KestrelError::InvalidRequest`,
including empty input and cached fetch paths. Cached fetch validates before
cache I/O. Transport and warm-up durations also validate deadline capacity.
These representability checks are not practical memory or latency budgets;
callers should still choose limits appropriate to their workload.

## Optional fetch score threshold

`--min-fetch-score SCORE` defaults to disabled and requires fetching. With the flag, positive-IDF BM25
scores doubled title plus snippet over the complete deduplicated pool contributed
by each query, before any gate rejection, pre-ranking or fetch-candidate truncation.
It uses the evidence tokenizer shared with the experimental snippet policy.
Scores remain internal and never populate or replace content-only `bm25_score`.
Scores depend on corpus size/composition and query terms, not a universal relevance
scale; choose a threshold only after evaluating useful-source retention.

The comparison is inclusive (`score >= SCORE`). Finite nonnegative numbers are
accepted, including zero (which keeps zero scores); negatives, NaN and infinities
fail before requests with usage status 2. `--no-fetch`
conflicts with this flag, also with status 2, stderr explanations and empty stdout.
Standalone `fetch` has no query and does not accept the option.

With the gate enabled, the CLI passes the same trimmed, deduplicated query list
to search and later stages. Whitespace-only queries are discarded; at least one
nonempty query is required. The library gate shares search's normalization, so
padded queries cannot lose their provenance match or gain an unrelated query's
score. No query or provenance field is rewritten by filtering.

Scoring uses tokenized original query text, without a Boolean query parser.
Operators, negated terms and site-expression words can contribute scores. This
intentionally replaces the old affirmative-only scoring; site/exclusion-only
queries no longer automatically bypass the gate. Queries with no lexical tokens
bypass it with a stderr diagnostic. Shared URLs
qualify if any contributing query passes or bypasses, preserving their original
query/engine fields, source occurrences and relative order. The reusable
`ranking::filter_fetch_candidates` helper treats candidates without matching
provenance as belonging to all supplied queries; library search/fetch APIs remain
separate and unchanged. Its CPU work is synchronous; the CLI runs it on a blocking
worker rather than the async executor.

Filtering applies even below the fetch cap and without `--pre-rank`. The existing
pre-rank policy runs afterwards only if survivors exceed the cap. Rejected URLs
leave both the fetch pool and final output. `--no-rank` and all fetch-compatible
ranking policies remain valid; final body ranking can still remove survivors.
Provider stopping is unchanged, so raising only the fetch cap or threshold does
not collect replacement candidates. There is no threshold relaxation or refill.
All-rejected searches perform no page/cache work and return a successful empty
result. Cache keys, PDF handling, final ranking and public JSON schemas are unchanged.

Stderr reports rejected candidates and contributing queries that bypassed the gate.
Existing benchmark fields remain; an enabled gate adds `fetch_score` timing and
`after_fetch_score`, `fetch_score_rejected`, `fetch_score_bypassed_queries` counts.
Scoring occurs between provider search and page fetching, outside their network
budgets; these flags are not total process-latency limits. Regenerate the installed
skill with the updated binary. Example (0.1 is illustrative, not calibrated):

```sh
kestrel search "rust async" --min-results 15 --fetch-candidates 8 --min-fetch-score 0.1 --no-rank
```

## Advisory content quality

Successful fetch means text was extracted, not that it supplies useful evidence.
Known shell-only bodies remain successful text/JSON responses; search retains
body, title, snippet, URL and provenance and applies its existing ranking policy.
Default JSON diagnostics expose the advisory quality assessment. The library and opt-in
search benchmark artifacts expose versioned advisory assessments. HTML extraction
may recover the first explicit article when the originally selected body consists
entirely of recognized shell messages. See [policy and evaluation](content-quality.md).
