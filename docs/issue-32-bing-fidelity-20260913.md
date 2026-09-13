# Issue #32: current retrieval-fidelity baseline

Issue: https://github.com/rafaelpierre/kestrel-rs/issues/32

Date: 2026-09-13. Production base: `a8fe1e1`. Branch:
`codex/32-current-fidelity`. Only the opt-in experiment, its tests and documentation
change. Production search, CLI/library contracts, transport defaults, generated
skill, dependencies and MSRV are unchanged.

## Decision

Keep #32 open for independently controlled browser/session/network comparison.
The current adapter still receives unrelated organic entries for complete queries.
Current portable filtering suppresses many of them, but also rejects judged-useful
previews. Increasing the portable result minimum from five to twenty did not
improve useful coverage in this small sample. Neither observation justifies a
production default change or an upstream/intermediary attribution claim.

The former fallback/quorum experiments have been superseded by #44, #54 and #68.
The [historical report](issue-32-bing-fidelity.md) remains historical evidence.

## Method and reproducibility

Two windows, starting 13:09:32 and 13:10:32 UTC, completed by 13:10:50. Six fixed
query intents, four network variants, 48 observations total. Twelve isolated Bing
responses also yield 24 paired native/portable views; these are not extra network
searches or independent latency samples. The query manifest is unchanged.

The host was arm64 macOS (Darwin 25.6.0). The fixed profile was Chrome 146/macOS/
en-GB, with no explicit region, reused clients without a cookie jar, five-second
budgets, and no page fetching or ranking. Fanout used all nine default providers,
concurrency ten, production retries and explicit result minima. Client pools are
reused within a window and recreated for the next. Variant order rotates by query;
the same order repeats in both windows. The small fixed query set, time/order
confounding and shared upstream state preclude causal performance comparisons.

The Rust test binary was frozen before either window. Its SHA-256:
`6e1e19de856faef59c746e81373c007e1698bc0801a6e12d68c22c39d3b73404`.
Manifest SHA-256:
`6be93ab7539557fc7c1f4d49ed9419f49be91122c734cf781ccf0161f85d0fa5`.
Tracked diff SHA-256 at build:
`2d0632529fdc7d16cc20ac67b3a1d930182125bba2dcd95566e801e0011fd08f`.
The final report was written after the run; the frozen binary identifies the
measured implementation, not a release binary or the final documentation tree.

Local ignored evidence is under
`benchmarks/results/issue-32-current-20260913/`: `schedule.json`, both
`window-*/runs.json`, opt-in raw HTML, `judgments-reviewed.json`, `scores.json`, and
six sanitized `browser/*.json` observations. The empty judgment template is
preserved separately. Raw captures and generated artifacts are not committed.
Response hashes identify decoded HTML, not compressed wire bytes. Initial URLs
retain exact query encoding; final URLs keep only q/cc and result destinations
lose query strings/fragments. These removals can collapse distinct destinations.

The updated [runner instructions](../benchmarks/bing-fidelity/README.md) include
an independently reproducible second-environment procedure. The user requested
local work now and preparation of that procedure; no second environment was used.
No proxy environment variables were present, but system proxies/VPNs/egress were
not independently inspected. Absence of those variables does not exclude an
intermediary. Browser cookies, identifiers and session stores were not inspected.

## Judged quality and latency

One Codex model analyst reviewed 73 distinct query + destination + title + snippet
keys against the declared intents. Eight ambiguous previews remain null; no
independent human adjudication or fetched-page accuracy check was performed.
An intent hit requires a preview/destination addressing the specific question,
not merely lexical overlap. Relevant means promising evidence for that intent,
not an endorsement of source correctness.

Usable coverage counts searches with at least one judged-relevant top-five entry
out of all scheduled searches, including empty/error outcomes. Conditional P@5
uses five slots per nonempty search; missing slots count as nonrelevant. Bounds
below allow each uncertain preview to be either relevant or nonrelevant. They are
judgment bounds, not statistical confidence intervals. The scorer leaves exact
P@5 null when judgments are missing; the bounds follow from the stated counts.
All coverage figures are exact under these judgments because each ambiguous run
already has a definite relevant hit.

| Variant | Nonempty / scheduled | Usable coverage | Conditional P@5 | p50 / p95 ms | Cancellations |
| --- | ---: | ---: | ---: | ---: | ---: |
| Raw isolated Bing | 12/12 | 4/12 | 0.267–0.317 | 276 / 732 | — |
| Native view of same Bing responses | 12/12 | 4/12 | 0.267–0.317 | not measured | — |
| Portable view of same Bing responses | 2/12 | 2/12 | 0.200 | not measured | — |
| Native fanout, minimum 5 | 12/12 | 10/12 | 0.717–0.817 | 89 / 722 | 58 |
| Portable fanout, minimum 5 | 8/12 | 8/12 | 0.400 | 1277 / 1483 | 0 |
| Portable fanout, minimum 20 | 8/12 | 8/12 | 0.400 | 1308 / 1555 | 0 |

Raw/native Bing has 16 definite relevant and three uncertain slots out of 60;
native fanout has 43 definite relevant and six uncertain slots out of 60.
Uncertain slots can repeat the same judgment key. Both portable fanout arms have
16 relevant slots out of 40. Portable Bing has two out of ten. There were no
whole-search errors or budget overruns; individual provider failures still
contributed zero results. These are debug-harness elapsed times, including parsing
and, for isolated Bing, paired-view computation and local raw-capture writing.
They are not isolated transport latency or production performance guarantees.
The p95 is the maximum of twelve samples using nearest-rank percentiles.

Both portable minima returned fewer than five accepted candidates on every query;
provider work exhausted rather than hitting either minimum. Thus the equality of
coverage does not test whether a higher minimum helps when five are reachable.
The native fanout's higher observed coverage is not evidence to disable portable
constraints: its requests occurred at different times and accepted unrelated
results on two searches. It also changes the documented query contract.

## Response-level and filtering evidence

All twelve isolated responses were HTTP 200 over HTTP/2, with complete queries in
the document titles and `Cache-Control: private, max-age=0`. That header alone
does not identify a cache or exclude intermediaries.

In window one, moon/tides returned relevant pages, but Rust returned game/general
language pages; Tokio returned Tokyo/band/general-runtime pages; the quoted Rust
error returned dictionary definitions of “use”; the site query violated docs.rs;
and PostgreSQL returned general product/download pages. In window two, the
unchanged Rust and Tokio baseline improved, while quoted/site/PostgreSQL failures
persisted. These temporal improvements cannot be credited to an implementation
change or to browser warming, personalization, encoding or transport.

For example, the window-one Rust decoded response hash is
`08e4960d402953e65e37d75b3fba360277e350b187c7d603a9b53df3dc38e39f`;
its first entries are the Rust game homepage and Rust language homepage. In
window two, hash
`e78d41cc985444d82f55a7ebd819e69147811985f331edfadb0f6f41449f7ea1`
contains explicit E0382 repair pages and the Rust error-code documentation.
The existing request/parser regressions and current raw-body captures support
response-level mismatch rather than dropped query words or invented parser output.
They do not prove all possible parser layouts correct.

Paired filtering removes 14 of the 16 definitely relevant raw Bing top-five
entries, counted across both windows: four moon results in each window, all five
Rust results in the second, and the definite Tokio hit in the second. Only the
moon query retains a relevant accepted result. For example, the Rust error-code
preview explains using references without containing every literal query term.
This is the documented portable contract's recall cost, not proof that those
pages satisfy every requested keyword in their full content. Native site filtering
retains one general docs.rs Tokio result on each site query, but it does not meet
the specific Receiver intent. Nonempty native output is therefore not sufficient.

## Browser comparison and attribution limits

The normal in-app browser was navigated to each exact isolated q URL, without cc.
Sanitized rendered captures were recorded between 13:10:14 and 13:12:06 UTC, so
some overlap the adapter windows and others follow them. They are not precisely
simultaneous matched trials. Moon, Rust, Tokio, quoted and PostgreSQL rendered
views contained on-topic destinations; the site view still showed Tokyo/band/
general-runtime entries instead of the requested docs.rs Receiver documentation.

The initial Rust accessibility view showed game/general-language entries; a later
DOM read on that tab showed E0382 results. Timing for the initial view was not
saved as a separate artifact, so this is a qualitative observation, not a timed
DOM-transition experiment. Some offscreen entries yielded empty rendered text;
these are missing observations, not proof that the browser omitted content.
Browser snapshots were not included in the adapter quality metrics.

Browser locale, session state, transport, redirect chain and egress equivalence
were not controlled. Rendering differences do not establish that JavaScript,
cache warming, upstream generation or any particular intermediary caused the
mismatch. No browser network-response capture was available through this surface.
The site failure corroborates that the symptom is observable outside Kestrel's
parser; fully controlled attribution and a second network remain outstanding.

## Validation and remaining work

- Four focused Rust harness tests pass, including native/portable paired filtering,
  current minima/budgets, and existing sanitized fixture replay.
- Seven Python scorer/sanitizer tests pass. Regressions keep failed captures in
  paired denominators, prevent replay latency/request double counting, reject
  missing paired views and reject incomplete parent schedules.
- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  and `cargo test --all-features` pass: 176 tests, four existing live/manual tests
  ignored. The matrix was separately invoked explicitly for the two live windows.
- Production executable/dependencies/packaging are unchanged; a release build and
  temporary generated-skill installation are not required for this test/report
  change. The installed skill's CLI and search behavior remain unchanged.

Next: reproduce on an independent network with matching source/configuration and
controlled browser/adapter timing, retain sanitized redirect/request/response
correlation where the environment supports it, and adjudicate ambiguous previews.
Only then reconsider a transport/retrieval mitigation. Do not close #32 or change
production policy based on this baseline alone. RTK.md was absent in the checkout.
