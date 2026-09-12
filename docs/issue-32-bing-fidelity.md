# Issue #32: Bing retrieval fidelity investigation

Issue: https://github.com/rafaelpierre/kestrel-rs/issues/32

Worktree: `kestrel-rs-issue-32`, branch `codex/issue-32-bing-fidelity`, based on
`95899d1`. Investigation date: 2026-09-12.

## Follow-up: budgeted CLI default

At the user's request, the CLI now selects fanout quorum 1 when `--search-budget`
is supplied without an explicit `--mode`. An explicit quorum overrides 1.
Explicit modes preserve their behavior: fanout without a quorum waits for all
providers within the budget, and fallback stays ordered. Unbudgeted defaults and
library `SearchOptions` are unchanged. Fallback removal belongs to a separate PR.
The investigation below describes the earlier measured behavior; no further
upstream investigation is part of this follow-up.

## Result and decision

Unrelated results were reproduced in actual response bodies, and an exact
site-query URL returned unrelated Tokyo/band results in the normal in-app browser
as well. The adapter faithfully extracts those entries. This failure is not
explained by Kestrel truncating the query or replacing results with the page title.
The responsible upstream/intermediary mechanism remains unproven.

Do not change default transport, encoding, locale, provider order or relevance
policy on this evidence. Transport impersonation, omitting cc, adding the
observed `form=QBRE` parameter and encoding spaces as `%20` showed no relevance
advantage over their contemporaneous baselines. No token-overlap gate was added.

The existing opt-in `--mode fanout --provider-quorum 1 --search-budget 3` mitigated
sequential fallback starvation in the final local windows. It does **not** detect
unrelated Bing results and is not a retrieval-fidelity fix. Full fanout added
latency without better coverage in these measurements. Keep both as user choices.

## Implemented

- Extracted the production Bing request builder and added complete-query
  construction regressions for multiword, quoted, site, Unicode and delimiter
  inputs, with empty and explicit regions.
- Added a synthetic query-echo/unrelated-results fixture and a deterministic
  regression demonstrating nonempty results stopping fallback/satisfying quorum.
- Added three sanitized live structural excerpts: unrelated quoted-query
  definitions, relevant Tokio documentation, and site-violating Tokyo results.
  Parser regressions verify their entries and existing site filtering.
  `tests/fixtures/providers/bing-live-provenance.json` records timestamps, request
  context and original-response hashes.
- Added an ignored live harness with a fixed browser header profile, controlled
  request variants, production orchestration, three-second budgets and per-search
  sanitized observations. The harness is test-only; the subsequent CLI change is described above.
- Added a repeatable scheduled runner, frozen executable/hash, explicit completion
  markers, reviewable relevance-judgment templates and coverage/precision/latency
  scoring. Tests guard missing judgments, failure denominators and sanitization.
- Added a raw-response excerpt tool and usage instructions in
  `benchmarks/bing-fidelity/README.md`.

## Experiment scope and results

The six-query manifest includes the three examples from #32 plus a quoted Rust
error, a docs.rs site query and a PostgreSQL EXPLAIN query. It is **not** claimed to
be the original #29 six-query manifest, whose raw pilot artifacts were unavailable
in the inspected worktree. No page fetching/ranking was performed. Judgments are
one analyst's assessment of title/snippet/destination against the declared intent;
they do not verify page content correctness or establish general provider quality.

The fixed-profile adapters use Chrome 146/macOS/en-GB headers and reused clients
without a cookie jar. The isolated transport trials use one attempt. The three
orchestration trials use production retries, default engines/order and site
filtering. Provider errors/empty results remain in scheduled coverage denominators.

### Initial and controlled windows

At 12:43:21–12:43:36 UTC, both standard and impersonated Bing returned nonempty
results for 6/6 queries but useful top-five results for only 2/6 (conditional
precision@5 0.333). Fallback, quorum 1 and full fanout each had useful coverage
2/6; site filtering removed the site-violating results.

At 12:44:39–12:44:57 UTC, all four transport/region/form variants had useful
coverage 3/6 and precision@5 0.467. Tokio had become relevant on the unchanged
baseline too. At 12:48:23–12:48:27 UTC, both `+` and `%20` trials had useful
coverage 4/6 and precision@5 0.633. The quoted-error query had become relevant
on both variants. These temporal changes cannot be credited to a tested change.

### Frozen-binary repeat

Two windows were scheduled 60 seconds apart, starting 12:50:24 and 12:51:24 UTC,
and finished by 12:52:16. The same frozen binary ran all 84 observations, 12 per
configuration. The site query remained unrelated in isolated Bing and was removed
by application filtering. The other five queries returned relevant results.

| Configuration | Nonempty / scheduled | Useful coverage | Conditional P@5 | p50 / p95 ms |
|---|---:|---:|---:|---:|
| Standard Bing, isolated | 12/12 | 10/12 | 0.800 | 302 / 443 |
| Impersonated Bing, isolated | 12/12 | 10/12 | 0.800 | 342 / 580 |
| Bing without region, isolated | 12/12 | 10/12 | 0.800 | 271 / 630 |
| Bing with browser form, isolated | 12/12 | 10/12 | 0.800 | 263 / 523 |
| Default fallback | 0/12 | 0/12 | undefined | 3003 / 3011 |
| Fanout quorum 1 | 10/12 | 10/12 | 0.960 | 283 / 3003 |
| Full fanout | 10/12 | 10/12 | 0.960 | 3003 / 3003 |

Quorum 1 cancelled 19 provider searches across these 12 requests. Full fanout
cancelled none by quorum and mostly waited until the deadline. The few
milliseconds above 3000 include timer wakeup and bookkeeping. Higher conditional
precision in fanout is partly explained by site filtering removing the bad query
from the nonempty denominator; useful coverage is the same as isolated Bing.
Fallback's all-provider errors do not retain detailed per-provider diagnostics in
the current public report. Its observed budget exhaustion is consistent with
sequential starvation; do not treat missing diagnostics as zero attempts or claim
that this experiment identifies every stalled request.

## Browser observations and attribution limits

Browser results were captured for all six queries. Initial moon and exact
site/PostgreSQL navigations used the adapter's query and cc parameters. The Rust,
Tokio and quoted-error comparisons used the visible search box; those submissions
omitted cc and included browser form parameters. Browser session state, language,
transport fingerprint and egress equivalence could not be fully matched through
the available browser interface. Cookies and identifiers were not inspected.
Browser artifacts are rendered-DOM observations, not HTTP-response captures.

The exact site URL reproduced unrelated Tokyo/band entries in the browser,
corroborating response-level failure outside the adapter parser. Other browser
queries returned relevant results. Some adapter responses improved after browser
comparisons, including on the unchanged baseline, but this temporal association
does not establish warming, caching or personalization as the cause. Captured
Cache-Control was `private, max-age=0`; that alone cannot exclude intermediaries.
Redirect chains are unavailable in the experiment and explicitly recorded as null.

Therefore the fully matched browser/network acceptance criterion and exact
upstream-versus-intermediary attribution remain open. A second environment and
independently controlled session/network traces are needed before selecting a
production fidelity fix. This is an investigation result, not a claim that #32
is resolved.

## Evidence and validation

Sanitized observations, judgments and scores remain locally under the ignored
`benchmarks/results/bing-fidelity-20260912/` directory. The report uses `window-1`,
`window-2`, `encoding-1` and the two completed `repeat` windows (168 observations).
An earlier `scheduled` run stopped before its second window and is excluded from
reported comparisons; its partial evidence remains local. The completed schedule
records the executable and manifest hashes. Raw HTML is opt-in and uncommitted.

`cargo test --all-targets`: 68 passing tests, one explicitly ignored live test.
`cargo clippy --all-targets -- -D warnings`: passed. Five Python scoring/sanitizer
tests passed. The live test was explicitly run for the measurements above.

The referenced `RTK.md` was not found in the checkout, its checked parent
directories or the local Codex directory.
