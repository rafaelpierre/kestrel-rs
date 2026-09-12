# Issue #40: Felo and Manus feasibility

Branch: `codex/issue-40-felo-manus` (base `95899d1`).
Status: investigation started; production integration is gated on evidence.

## Implementation plan

1. Inspect Felo and Manus independently: public entry page, browser search/task
   interaction, actual HTTP responses and frontend request contracts. Identify
   authentication, cookies, JavaScript, streaming and asynchronous dependencies.
2. Implement a bounded, reproducible public-entry HTTP probe with structured
   outcomes, status, latency, response size and body fingerprint. Never interpret
   landing-page links or generated text as source results. Keep raw traces local.
3. For each observed keyless retrieval path, implement the minimum request and
   source extractor; capture sanitized real fixtures. Test full-query preservation
   (multiword, quotes, punctuation, Unicode, `site:`), missing fields, ordered
   citations, valid empties, challenges, malformed data and errors. Record blockers
   if no such path is observed; do not invent endpoints or success fixtures.
4. Use matched general, coding and fresh-information queries from the #29
   benchmark, plus operator cases. Repeat fresh and reused sessions in at least
   two time windows with controlled pacing. Compare existing providers using the
   same queries/budgets. Report explicit attempt/search denominators, nonempty and
   usable coverage, judged precision@5, success/failure p50/p95, timeout, empty,
   challenge, HTTP-error and rate-limit counts. Small samples are preliminary.
5. Assess each provider against `docs/search-providers.md`: fallback/fanout,
   deadlines, cancellation, retries, provenance and canonical-URL deduplication.
   Document supported/unsupported region and recency semantics independently.
6. Record separate go/conditional-go/no-go recommendations, maintenance risks,
   dependencies and effort estimates. Only a viable recommendation leads to
   opt-in provider adapters, Engine/CLI/library registration and documentation.

Estimated investigation effort: 2–4 engineering days plus separated measurement
windows. If viable, allow 1–2 additional days per adapter, contingent on protocol
complexity. These estimates are provisional until a retrieval contract is captured.

## Initial implementation scope

Start with public-entry access diagnostics and this evidence log. This does not
constitute a search adapter or establish provider availability. Acceptance criteria
remain open until the retrieval, fixture and repeated-measurement work is complete.

## Observations — 2026-09-12, approximately 12:51–12:54 UTC

Environment: local macOS arm64, Rust 1.96.0; browser: Codex in-app browser.
Network egress region was not verified. No credentials were supplied.

### Felo

- Public entry: <https://felo.ai/search>. The interactive page has an anonymous
  query box and a Login button. A promotional dialog required dismissal.
- Submitted `Rust E0382 use of moved value` through the visible Send button.
  The page displayed “Checking security”, then proceeded automatically to
  `/search/<thread-id>` without manual challenge completion or login.
- The completed answer had an **11 Sources** panel containing numbered titles,
  display URLs and snippets. The generated answer was Chinese despite the English
  query. Generated prose must not be substituted for source snippets.
- Genuine citation links included
  <https://users.rust-lang.org/t/how-to-solve-this-used-of-moved-value-issue/68138>
  (source 1) and <https://doc.rust-lang.org/error_codes/E0382.html> (source 8).
  A small manually transcribed browser observation is preserved alongside this
  report. It is not an HTTP fixture or proof of an extraction contract.
- One browser search completed with sources (1/1 exploratory submission). No
  controlled latency or top-k relevance measurement was made. No inference about
  reliability, clean browser sessions, operator semantics or HTTP viability follows.
- The separate official <https://openapi.felo.ai/> surface advertises an API key
  for search and other tools. That credentialed alternative is outside keyless
  adapter scope; pricing and wire contracts remain unverified.
- Provisional disposition: **continue investigation**, with a conditional-go
  candidate only if anonymous source retrieval can be reproduced using Kestrel's
  HTTP clients within budget. No production go recommendation yet.

### Manus

- Public entry: <https://manus.im/>. The landing page presents a task-oriented UI;
  selecting Sign in navigated to `/login` and offered account authentication.
- No anonymous search-results contract was established. Clicking Sign in proves
  only the observed route; it does not prove every Manus feature requires login.
- Its separate official <https://open.manus.ai/docs> page presents “Get your API
  key”, projects, webhooks, skills and agents. A keyless synchronous web-search
  endpoint, task polling protocol and source metadata remain unverified.
- Provisional disposition: **hold integration** until an anonymous retrieval path
  is demonstrated. This is not a final no-go based on a single landing-page visit.

### Direct HTTP access probe

`examples/provider_entry_probe.rs` is implemented without new dependencies. It
uses reqwest/rustls, a declared diagnostic user agent, a 10-second request timeout,
2 MB streamed-body limit, at most five redirects, and no retries or cookie store.
It emits one JSON record; provider failures are recorded in `outcome`, while CLI
usage/setup failures exit nonzero. HTTP status and heuristic challenge hints are
separate. Unknown bodies, including empty/JSON bodies, are never search successes
or valid search empties. Bodies, headers and final redirect identifiers are omitted.
The response fingerprint describes the complete body only when fully consumed.
This standalone diagnostic does not use Kestrel's randomized browser profiles.

| Provider | UTC | HTTP | Protocol | Seconds | Body bytes | Classification |
|---|---|---|---|---|---|---|
| Felo | 12:53:20 | 200 | HTTP/2 | 0.962 | 289938 | unvalidated entry response |
| Manus | 12:53:21 | 200 | HTTP/2 | 0.354 | 231713 | unvalidated entry response |

Denominator: one logical entry probe per provider, two total; redirects were not
observed, so two HTTP requests total. Neither is a search attempt. Zero observed
HTTP failures/timeouts; search success, valid empties, quality and percentiles are
**not measured**. These are one-window access observations only.

Reproduce from the worktree:

```sh
cargo run --example provider_entry_probe -- felo
cargo run --example provider_entry_probe -- manus
cargo test --example provider_entry_probe
cargo clippy --example provider_entry_probe -- -D warnings
```

Next: inspect Felo's actual frontend request/stream contract, capture sanitized
network fixtures and prototype source extraction; inspect Manus's visible task
entry and public documentation independently. Browser tooling used here exposed
DOM and console observations, not a network-response capture interface. Full query
encoding/operator tests, filter support, controlled fresh/reused sessions, matched
provider comparisons, cancellation/retry checks and final recommendations remain
pending. None of the issue's acceptance checkboxes is claimed complete.
