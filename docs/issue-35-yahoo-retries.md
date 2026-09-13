# Yahoo redirect and retry investigation — 2026-09-13

Related: [#35](https://github.com/rafaelpierre/kestrel-rs/issues/35),
[#29](https://github.com/rafaelpierre/kestrel-rs/issues/29),
[diagnostic implementation #42](https://github.com/rafaelpierre/kestrel-rs/pull/42).
Status: partial investigation; production retry policy unchanged. Do not close #35.

## Current reproduction

Six isolated Yahoo searches (two sequential sweeps of manifest q01–q03) failed.
All 18 application sends returned HTTP 500 after automatic redirects, at
`https://search.yahoo.com/_bv/v.gif` (query/identifier values omitted). All 18
bodies were empty; all challenge classifications were `unknown`; no Retry-After
was observed. Twelve retries recovered nothing. No transport error occurred in
this sample. These are provider failures, not successful empty searches or
failures to fetch downstream pages: page fetching was disabled.

Median logical provider time was 2,583.5 ms; median external CLI time was
4,174.5 ms. These include different work: the latter also includes initialization,
process overhead, serialization and instrumentation. Multiple other builds were
running on the host, so timings are not a clean performance comparison. The
three-second search budget is not an end-to-end process deadline.

During this session, before the CLI study, the same Yahoo entry point and `Australia capital`
query displayed organic Wikipedia and Britannica results in the Codex in-app
browser, with a privacy notice. No login, CAPTCHA, cookie transfer or challenge
solving was performed. The only browser exposed by the tool inventory was the
in-app browser. An ordinary Chrome/Firefox profile, HTTP status, full network
redirect chain, cookies, TLS negotiation and browser request headers were not
available through the inspected surface. Thus this is a same-host contrast, not
a controlled matched-browser experiment. It does not identify the root cause.

The CLI uses primp impersonation with a coherent randomized profile fixed for
its lifetime. This sample included Chrome 148 on Windows/macOS/Linux and Firefox
146 on Windows/Linux, with en-US/en-GB. All failed. Browser state, JavaScript,
HTTP/TLS behavior, proxy routing and server-side classification remain possible
confounders; the endpoint name alone proves none of them. Automatic redirects
are not separately counted by the current lifecycle recorder.

## Reproduction and provenance

Run from the feature worktree:

```sh
cargo build --release
python3 benchmarks/yahoo_retry_study.py --binary target/release/kestrel --output /absolute/new/private-directory
python3 benchmarks/yahoo_retry_replay.py /absolute/new/private-directory
python3 -m unittest discover -s benchmarks -p test_yahoo_retry_replay.py
cargo test --lib diagnostic_tests
```

The runner refuses an existing output directory. It retains exact argv, UTC
start time, exit status, external wall time, stdout/stderr, every available
provider capture and outcome, generated headers, and search artifacts. Production
request timeout remains 15 seconds, bounded by a three-second shared deadline;
the outer process timeout is 30 seconds. It requests 100 unique results to avoid
premature result-count cancellation and returns at most 20. No fetch, ranking or
page cache is used; upstream cache state is uncontrolled. Calls are separated by
250 ms and each starts a fresh process/client. No retained-client claim is made.

Base revision: `e7527719feff0265ea34ee7334f289f076162b76` (Kestrel 6.0.0).
Base tree: `900eba9b6c8058a0b075ea12c665b3ae0cd434f5`.
Release executable SHA-256:
`514df0ef84999eb6cf968e57d49a43a0b9797302cd86cc68c894033874258e36`.
Manifest SHA-256:
`cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.

Full local artifacts:
`/Users/rafaelpierre/projects/kestrel-rs-issue-35/benchmarks/results/issue-35/`.
`live-v1` is the immutable live run. `replay.json` retains an initial replay that
incorrectly omitted null-attempt-ID backoff intervals; **do not use it**.
`replay-v2.json` is the corrected analysis, protected by a regression test.
Raw files are uncommitted and may contain provider identifiers. The sanitized
observations here are the shareable evidence; one short run cannot establish
global availability, recovery probability or a statistically reliable speedup.

## Bounded policy evaluation

This is **offline prefix replay**, not an executed candidate policy. It retains
observed intervals up to the selected send cap, excluding queue/startup, omitting
backoff after the last allowed send, and preserving censoring. Backoff carries
no attempt ID, so the replay associates it by sequence order. Intervals are
integer milliseconds; their sum can differ slightly from total logical time.

| Send cap | Retained sends | Saved sends | Observed HTTP recoveries retained | Median retained intervals |
| --- | ---: | ---: | ---: | ---: |
| 3, current | 18 | 0 | 0 | 2,580.5 ms |
| 2 | 12 | 6 | 0 | 1,654.5 ms |
| 1 | 6 | 12 | 0 | 937.5 ms |

The medians use the mean of the two middle values; no tail percentile is claimed.
These arithmetic savings do not predict future elapsed times or server behavior.

Deterministic synthetic sequences cover persistent 500s, 500→200, 500→500→200,
and typed network failure→200 in replay. The real Rust transport regression
follows `/search`→`/_bv/v.gif` on a local mock server, tests empty 500 exhaustion
and recovery on sends two and three, and checks Retry-After/status retention,
unknown challenge classification, actual redirect request count and raw empty
body capture. Its HTTP 200 recovery is an explicit **valid no-results page**,
not proof of useful search results. Existing typed connection-error, body-error,
TLS, caller/quorum cancellation and deadline/backoff tests remain applicable.

| Synthetic sequence | Cap 1 | Cap 2 | Cap 3 |
| --- | --- | --- | --- |
| 500, 500, 500 | failure | failure | failure |
| 500, 200 | loses recovery | recovers | recovers |
| 500, 500, 200 | loses recovery | loses recovery | recovers |
| connect error, 200 | loses recovery | recovers | recovers |

Synthetic elapsed-time fixtures use 25 ms per attempt plus 100/200 ms backoffs:
25/150/375 ms for caps 1/2/3. They are modeled values, not live measurements.
A provider cooldown could suppress more sends across calls but needs retained
client state, expiry, one half-open probe and concurrent probe suppression.
A fixed cooldown also suppresses successes that become available during that
window. It has not been implemented or measured here. Independent CLI processes
would not share a client-local breaker. No global 5xx retry removal is justified.

## Deadlines, Retry-After and remaining work

Current Yahoo retries HTTP 408/429/5xx and every send error, up to three sends;
standard transport retries a narrower set of typed send failures. Retry backoff
is 250–499 ms then 500–999 ms, plus scheduling overhead. Retry-After is observed
but not honored in scheduling. HTTP error-body failures retain status-based
retry rules; a successful-status body failure and oversized provider response
terminate without retry. Shared deadlines/caller/threshold cancellation can
interrupt sends or backoff; cancellation during backoff does not add a send.

A candidate Retry-After-aware implementation must parse both delta seconds and
HTTP dates, account for malformed/past values and clock skew, and never extend a
shared deadline. If the server's requested wait cannot fit, stop that operation;
future cooldown expiry must be explicit. Preserve diagnostics for the last
completed response and distinguish skipped probes from failures. This is design
work still required, not a claim that current code implements these semantics.

Remaining acceptance work under #35:

- Matched ordinary-browser/transport experiment and redirect-hop evidence to
  isolate the actual cause of the failure.
- Retained-client and multi-environment observations, including genuine transient
  recovery, before choosing a cap or cooldown.
- Executed bounded-policy/cooldown comparison with cancellation, Retry-After,
  half-open concurrency and recovery-loss measurements if a rollout is proposed.
- Passing the mandatory ten-question evidence gate before readiness/merge.

No CLI, library, search/fetch behavior, dependency, MSRV or skill contract changes
are introduced. The final executable's generated skill was installed into the
local artifact directory's temporary project and used for the evidence workflow;
existing installed skill files were not overwritten.

See [ten-question results](issue-35-evidence-gate.md) for independent acceptance
status and exact retrieval policy. Rust checks and evidence usefulness are
separate judgments.

## Validation results

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --all-features`: passed, 225 tests and five ignored.
- `cargo build --release`: passed; final binary hash matches the recorded run.
- Python prefix-replay regressions: three passed, including null-ID backoff,
  recovery loss, censored backoff before the cap and zero-attempt records.
- Initial parallel diagnostic subset: 15 passed and two existing 200 ms
  deadline-phase tests failed before reaching the intended phase. The new Yahoo
  test passed. All 17 passed serially, and the later full parallel suite passed.
  This matches the observation tracked by
  [#124](https://github.com/rafaelpierre/kestrel-rs/issues/124); host contention is
  plausible, not a proven diagnosis. Both logs are retained.
- Generated skill installation into the temporary project succeeded; the
  generated option reference matches the tested binary's search/fetch help.
- Mandatory evidence gate: **NOT PASSED (3/10)**. Draft only; no merge.

[Notion findings](https://app.notion.com/p/3da77f15889081ce8d7accd159807a02).
