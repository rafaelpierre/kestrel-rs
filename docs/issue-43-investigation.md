# Issue #43: slow fanout results

Investigated on 2026-09-12 at commit `95899d12c70021daab8152c85b0756c7147879a2`.
Issue: https://github.com/rafaelpierre/kestrel-rs/issues/43
Branch: `codex/issue-43-fanout-latency`.

## Reproduction

Built the current source and ran the reported query against live providers:

```sh
kestrel search 'what is ML' --mode fanout --output json
```

The search phase took **46.475 seconds**. Fetching added 1.007 seconds and ranking
added 9 milliseconds. The provider diagnostics explain the delay:

| Provider | Elapsed | Results | Outcome |
| --- | ---: | ---: | --- |
| Bing | 364 ms | 10 | Success, no retries |
| Yahoo | 1,780 ms | 0 | HTTP 500 after two retries |
| DuckDuckGo | 46,474 ms | 0 | Request failure after two retries |

The command ultimately returned five ranked results after extracting seven of ten
pages. Useful search results were available almost immediately, but were held
until DuckDuckGo exhausted its request attempts.

## Cause

- `src/search.rs:227`: fanout uses `FuturesUnordered`, so providers already run
  concurrently. Increasing search concurrency will not fix this single-query case:
  the default concurrency is five and there are only three providers.
- `src/search.rs:283`: `collect_fanout` drains all provider futures unless an
  explicit successful-provider quorum is supplied.
- `src/model.rs:156`: both the quorum and total search budget default to `None`.
- `src/search.rs:29` and `src/search.rs:730`: standard providers get a 15-second
  request timeout and up to three attempts, plus retry backoff. This permits
  approximately 46.5 seconds of waiting for one failing provider. Yahoo has a
  separate three-attempt retry loop with the same request timeout.
- `src/cli.rs:344`: the CLI awaits the complete search report before printing the
  result count or starting extraction, making this wait look like a stall.

Fetching is also enabled by default, with up to 15 candidate pages for the default
top five, five concurrent requests, and a ten-second per-page timeout. That can add
latency on other runs, but it was not the main cause in this reproduction.
`--timeout` controls page fetching, not provider searches.

## Existing controls verified

Two additional live runs used the same query with page fetching disabled to isolate
provider latency:

| Additional flags | Search time | Outcome |
| --- | ---: | --- |
| `--search-budget 5 --no-fetch` | 5,001 ms | 10 Bing results retained; DuckDuckGo hit the deadline |
| `--provider-quorum 1 --search-budget 5 --no-fetch` | 505 ms | 10 Bing results retained; two outstanding providers cancelled |

Each command returned five results. These are individual live measurements, not
a statistical benchmark; provider availability, content, and latency can change.
The no-fetch runs return search snippets and do not perform page-body ranking.

For a bounded search that still fetches and ranks pages:

```sh
kestrel search 'what is ML' --mode fanout --search-budget 5 --fetch-budget 5
```

For the fastest snippet response, the verified command is:

```sh
kestrel search 'what is ML' --mode fanout --provider-quorum 1 --search-budget 5 --no-fetch
```

## Recommended fix

Give CLI fanout a finite default search budget, reusing the existing deadline and
partial-result handling. Five seconds is a starting point to evaluate, not a
benchmark-derived optimum. Keep an explicit way to request a longer or unlimited
wait, and document that the budget covers provider search rather than extraction.
Preserve the library's existing default unless changing its behavior is intended.

A default quorum of one would be faster but would reduce every successful fanout
to the first returning provider. Keeping quorum opt-in preserves the opportunity
to combine providers within the time budget. A quorum of two alone would still
wait for the failing providers in this reproduction because only Bing succeeded.

Before implementing a default change, add deterministic regression coverage for
a quick successful provider plus a never-completing provider: verify bounded
completion, retained results, and deadline diagnostics. Cover explicit budget
overrides and the all-providers-fail case. Also consider progress reporting for
completed providers so a longer requested wait remains understandable.

## Artifacts

Raw benchmark reports and outputs are stored locally under
`target/issue-43-investigation/` (ignored by Git).

## Implementation follow-up

CLI fanout now defaults to a five-second search budget, with explicit durations
and `--no-search-budget` supported. The library search budget remains opt-in. After rebasing onto the fanout-only
v2 release, the five-second budget applies to all CLI searches. Search, fetch, and parse concurrency now default to 10 in both the CLI
and library. Regression coverage checks deadline retention and diagnostics,
total failure, CLI budget selection, and concurrency defaults.

Validation: formatting, Clippy with warnings denied, and all 65 tests passed.
The reported command with the new defaults returned five ranked results in
approximately 4.1 seconds (1.993 seconds searching, 2.004 seconds fetching).
DuckDuckGo failed quickly on this run, so this live result is a smoke test;
the deterministic stalled-provider regression verifies deadline behavior.
