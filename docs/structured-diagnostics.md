# JSON completion and evidence diagnostics

Successful `search --output json` and `fetch --output json` include a
`diagnostics` object with `schema_version: 1` by default. Add `--no-diagnostics`
to either command to restore the previous envelope exactly (search: `results`,
`elapsed_seconds`; fetch: `url`, `content`, `elapsed_seconds`). This intentional
breaking default extension does not alter result objects, text output, search,
fetching, ranking, library report types or benchmark artifacts. The opt-out has
no effect on text output. Regenerate installed agent skills with the new binary.

## Search completion

`diagnostics.search.queries` contains normalized, nonempty, deduplicated queries
in their input order, identified by zero-based `query_index`; query text is not
repeated. Each entry has `unique_accepted` (canonical URLs contributed to that
query before fetching/ranking), `minimum_reached`, `deadline`,
`providers_exhausted`, and `all_failed`.

These are independent observed conditions, not a precedence-ordered reason:
minimum reached and providers exhausted can both be true. Deadline means at least
one provider reported the shared search deadline; it is not inferred from command
wall time. A deadline and minimum can both be observed. Exhausted means every
scheduled provider for that query finished without deadline/cancellation; errors
count as finished. All-failed means no retained candidate and no successful
provider, while a successful empty or all-filtered response is not all-failed.
The aggregate contains `query_count`, counts of each condition with `_queries`
suffixes, `all_minimum_reached`, and `budget_exhausted` (any observed deadline).
A query's count includes a shared URL once; summing query counts can exceed the
cross-query `unique_accepted` count. `minimum_per_query` is the collection target,
not a final-result or evidence guarantee.

## Provider counts and timings

`search.provider_outcomes` counts every scheduled provider/query by outcome
(including detail rows omitted by the bound); absent keys mean zero.
`search.providers` contains one row per scheduled provider/query, in scheduling
order. `engine` and `query_index` identify it. `outcome` preserves the report's
known outcomes: results, empty, filtered_empty, deadline, cancelled_min_results,
cancelled_quorum, cancelled_caller, response_too_large, challenge, unrecognized,
request_error; future/unrecognized values become unknown.
`response_completed_successfully` is separate from `retained_occurrences`:
a cancelled response may still contribute evidence candidates.

Counts use only each provider's final observed cumulative snapshot, never sum
streaming updates or benchmark snapshots. `raw = rejected + accepted_snapshot`;
rejected describes provider normalization/filtering, before cross-provider URL
fusion. These are observed parser records, not an estimate of unread response
records. Error snapshots may include earlier records subsequently retracted.
`rejected_response_snapshot` counts accepted snapshot records whose response
ultimately failed (excluding retained deadline/minimum cancellations).
`retained_occurrences` instead counts final source occurrences in the merged
search results; `unique_accepted` counts those merged URLs. Snapshot publication
and cancellation can race, so accepted snapshots are observations, not a promise
of contribution. Do not add occurrences to unique candidates or treat cancelled
records as failed solely because `response_completed_successfully` is false.

Provider `elapsed_ms` is existing total logical elapsed time, including queueing
and retries. `timing_censored` is true for deadline/cancellation, false for
successful completion, null for errors whose interrupted phases are unavailable
in SearchReport. It is not a per-attempt latency distribution. `retries` reuses
the existing application retry count. `cancelled_minimum_or_quorum` reuses the
report's cancellation counter and does not include deadline outcomes. Detailed
attempt phases remain in the separate opt-in lifecycle trace interface.

## Candidate stages and evidence

`diagnostics.candidates` reconciles the unique search pool with later stages:
`after_fetch_score = unique_accepted - fetch_score_rejected`; `after_selection`
is the pool surviving the score gate and candidate cap; `not_selected` is the
difference from unique_accepted. `after_ranking` precedes `returned` (top-k).
Without fetching, after_selection is the original pool. No stage refills slots.

`diagnostics.evidence` counts over the entire pre-ranking candidate pool, not only
returned results: `selected` is zero for no-fetch, otherwise after_selection;
`scheduled` excludes URLs skipped by the existing case-insensitive `.pdf` rule;
`completed` includes successful, failed and cached fetch outcomes; `extracted`
counts retained bodies, including cache hits. `states` counts all original unique
candidates, including `not_selected`. The possible states are:

- `no_fetch`, `not_selected`, `skipped_pdf`;
- `extracted`, `cache_hit`, `empty_extraction`, `unsupported_content_type`,
  `response_too_large` (legacy report outcome), `request_failed`;
- `fetch_deadline` for selected scheduled pages missing an outcome when the fetch
  budget expired; `unknown` when no available report explains the missing outcome.

`budget_exhausted`, `cancelled`, and `cache_hits` reuse FetchReport. A missing
page's timing is null, never a zero-duration completed request. Completed page
`timing.total_ms` reuses the report; `timing.censored` is null for request failures
(the report cannot distinguish timeout from other failures), otherwise false.
Byte-cap extraction can finish successfully while the page is incomplete:
`byte_cap_reached` reports that separately (unknown/null for missing diagnostics).

`quality` counts advisory `unflagged`, `boilerplate_only`, and `unknown` states
over after_selection, including missing bodies. Absent count-map keys mean zero.
These reuse [content quality version 1](content-quality.md), bounded to 32,768
UTF-8 bytes per assessment. No state certifies usefulness: `usable` is null and
`usefulness_unknown` equals extracted. Even an unflagged body needs inspection.

`diagnostics.pages` describes the first 256 original unique candidates, with
zero-based `candidate_index` before selection and `returned_index` (null if absent
from final results), state, quality, timing and byte-cap observation. Use
`results[returned_index].url` for returned pages. Unreturned candidate identities
are intentionally not exposed; this is not a hidden full result list.

## Standalone fetch, bounds and failures

Fetch diagnostics have schema_version, state, quality, usable (null), timing,
byte_cap_reached and budget_exhausted. They assess the body before adding the
Source wrapper. Fetch has no total fetch-budget flag, cache or search completion
fields. Successful shell-only content still returns success.

Search emits at most 128 query rows, 128 provider rows and 256 page rows, with
`queries_omitted`, `providers_omitted`, and `pages_omitted` reporting truncation.
Aggregate counts cover all inputs regardless of truncation. Diagnostic fields
contain no raw bodies, query/URL strings, cookies, credentials or verbose error
messages. The bounded extension needs no trace directory or persistence. The
ordinary results/URL/content fields still contain the requested output.

CLI parser validation and argument conflicts still exit 2 with stderr and empty
stdout. Runtime validation (for example, only whitespace queries) and failures,
including all providers failing across all queries, client initialization,
selection/fetch failures, and standalone fetch without extractable text, still
exit 1 with stderr and no JSON error envelope or success completion line. A
successful empty search emits diagnostics and exits 0; in a multi-query success,
a failed query is visible via all_failed. On failures there is no new structured
error report; consult stderr or explicitly enable the existing traces.

## Caller examples

```sh
kestrel search 'rust ownership' --no-fetch --output json > discovery.json
# Retry with a larger search budget only if collection was deadline-limited.
jq '.diagnostics.search | {budget_exhausted, all_minimum_reached}' discovery.json
# Inspect snippets and choose a URL; fetching it supplies the missing body.
jq '.results[] | {url, snippet}' discovery.json
kestrel fetch 'https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html' --output json
# Keep strict existing envelope consumers working during migration.
kestrel search 'rust ownership' --no-fetch --output json --no-diagnostics
```

Existing environment capture is independent: `KESTRELSEARCH_PROVIDER_TRACE_DIR`
writes lifecycle traces, including failed searches; both
`KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR` and `KESTRELSEARCH_BENCHMARK_RUN_ID` enable
successful search artifacts. Standalone fetch does not write search artifacts.
Those potentially sensitive formats remain unchanged; see
[provider diagnostics](provider-diagnostics.md). Transport/warm-up/batch-fetch
controls remain library-only; this change does not expose new controls for them.

Candidate evidence is summarized before ranking; final result positions are added
after truncation. JSON diagnostics do not retain a duplicate set of candidate page
bodies. A full pre-ranking snapshot is retained only when both benchmark artifact
environment variables are present; that configuration is resolved once.

## Discovery retries

Provider rows now add one-based `discovery_attempt`; `retries` still counts HTTP
retries within that attempt. Provider rows/outcome totals retain all scheduled
attempts, in scheduling order; query completion conditions use the latest
observation per provider/query. `rate_limited_deadline` reports an attempt
deadline after observed HTTP 429 or Retry-After guidance, and does not authorize
another discovery retry. Like `deadline`, it can retain streamed candidates.
Earlier deadline rows remain even when a later attempt succeeds; they do not
make that query's latest completion deadline-limited. Existing bounds apply.
See [policy and compatibility](discovery-retries.md).
