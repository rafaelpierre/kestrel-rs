# Empty-discovery recovery: issue #79

**No-go for a default retry rollout on this evidence.** Retain one longer
provider deadline when the user can afford it. The controlled transient fixture
shows a reason to consider an opt-in retry, but the matched live window never
triggered recovery and therefore cannot establish a live benefit. Delivery of
production retries/error presentation belongs to [#189](https://github.com/rafaelpierre/kestrel-rs/issues/189).

The [reproduction policy](../benchmarks/empty-discovery/README.md) defines four
arms with identical query text, quality criteria and maximum wall/work allowances.
No production behavior, public API, CLI flag, dependency or skill template changed.
This study builds on #75's matched-policy methodology, #76/#81's distinctions
between collection and final output, #80's initialization evidence and #125's
already shipped persisted progress. Logical empty discovery does not duplicate
crash recovery or page-body resumption.

## What the observations establish

The local fixture where every response takes 1.4 seconds favors preserving
in-flight work: one longer attempt avoids the repeated request. The fixture
where only the first request stalls favors restarting. Neither fixture estimates
how often that failure pattern occurs on the real web. Actual local server logs
show two TCP connections for both retry variants after cancellation; retaining
the client does not revive a cancelled HTTP response. Fresh retries also rebuild
the client. Connection counts are not TLS handshake measurements.

The initial fixture window is retained at `/tmp/kestrel-79-fixtures-v1`; the
repeat with runtime outcome/send-count assertions is `/tmp/kestrel-79-fixtures-v2`.
Both use the same executable and discovery policy. Version 2 adds source identity,
end timestamps and assertions, not a changed retrieval policy. Full stdout,
stderr, request inputs, lifecycle traces and server logs remain local.

## Live window: September 14, 2026

One matched round, ten canonical FTS queries, four arms, all nine engines,
minimum one, metadata only. Full records: `/tmp/kestrel-79-live-v2`.
Every arm returned nonempty results for all ten queries, sent 90 application
requests, and had zero missing lifecycle records. **Neither retry arm actually
retried.** Nonempty coverage therefore does not measure recovery effectiveness.

| Arm | Nonempty | Queries with a directly relevant candidate | Wall p50 | Wall p95/max | Sends/query |
| --- | ---: | ---: | ---: | ---: | ---: |
| Short | 10/10 | 6/10 | 0.402 s | 0.539 s | 9 |
| Long | 10/10 | 6/10 | 0.413 s | 0.583 s | 9 |
| Retry fresh | 10/10 | 7/10 | 0.411 s | 0.580 s | 9 |
| Retry reuse | 10/10 | 7/10 | 0.429 s | 0.577 s | 9 |

The extra directly relevant query in the latter arms is q06: configuration pages
arrived instead of the CLI landing page. Since these were all first attempts,
this is provider/window variation, not a retry effect. q03 returned Python
landing/download pages, q07 mixed the Rust game and language homepage, and q08
returned TRAPPIST-1 **e** results. They are nonempty but lack a directly relevant
candidate for the requested intent. q01/q02/q04/q05/q09/q10 had directly relevant
subject/reference/visitor pages in all arms. Individual URL/title/snippet
judgments, metadata hashes and rationales are retained in `relevance-v1.json`.
This relevance assessment does not certify the ten-question answer gate.

Nearest-rank percentiles include errors/empties; n=10 makes p95 the maximum.
Live ordering is rotated but not fully position-balanced. No concurrent study
calls or builds ran during the live window; upstream caches, network conditions
and unrelated desktop load remain uncontrolled. Do not interpret small time
differences as a performance improvement. Discovery timing is retained separately
from process wall time; no page fetching occurred in this matrix.

## Concrete API and policy recommendation

Expose structured query/provider completion on **every** outcome, including all
providers failing. Today `merge_outcomes(...)?` can return before `SearchReport`
is exposed. Production code must not parse concatenated error strings or use the
benchmark's on-disk trace workaround to decide retries.

Represent accepted discovery before fetch/rank. A recovery candidate requires
zero accepted candidates for that query plus deadline-exhausted eligible work.
A valid empty response is different from all-provider failure; an empty final
body-ranked list is neither trigger. Preserve successful query IDs and all source
provenance. Do not repeat a successful query or a challenged/hard-failed provider
because another provider timed out. Retain native query bytes and restrictions.

If #189 proceeds, make the decision explicit and keep it opt-in until matched
failure-rich live evidence supports the cost. Start from a finite `RecoveryPolicy`
with at most two calls, an absolute overall deadline shared with initialization,
backoff and nested retries, and a remaining application-send counter per
query/provider. Carry those counters into the inner retry loop; do not multiply
independent retry ceilings. A candidate study setting is 1 second, 250 ms backoff,
then at most 3.75 seconds, with a 6-second process allowance. These are experiment
parameters, not established optimal defaults or a change to today's CLI budgets.

Use one retained client. If the first request is merely slow and remains active,
continuing it under one longer absolute deadline avoids duplication. If it has
already been cancelled, a retained pool cannot preserve its work. A future soft
checkpoint API should expose pending work without cancelling it; its semantic
equivalence to one longer attempt needs validation before a separate abstraction.
Keep caller cancellation active during both backoff and requests, and preserve
multi-query global concurrency. Disabling the search budget must not secretly
introduce automatic retry budgets. Explicit budgets need a documented total vs
per-attempt contract before rollout.

The 429 fixture also shows existing inner retries do not honor its 30-second
Retry-After as a pause: two or three sends occur inside one call. The experimental
outer controller correctly avoids adding another call. #189 must account for
this nested behavior and honor retry guidance within the absolute cap; an
unaffordable Retry-After should end recovery. This study does not repair the
existing inner transport policy.

## Limits and remaining decision evidence

Actual agent-turn latency, production CLI parsing overhead, live TLS handshake
cost, multi-query runtime scheduling/cancellation races and a representative
frequency of retryable live empties were not measured. The experiment's fresh
process is the benchmark probe, not an actual model issuing a second CLI tool
call. The Python controller's elapsed time includes its own overhead; model
latency is unknown, not zero. Multi-query eligibility and cancellation exclusion
have deterministic unit coverage, not end-to-end scheduling coverage.

These limits justify the no-go; they are not waived acceptance evidence for a
production retry default. Keep #79 open for those remaining measurements and
share the harness/results with #189 rather than claiming its rollout complete.

## Controlled repeat and executable identity

The asserted v2 fixture matrix passed all 28 cells, including exact result
presence, send/server-count reconciliation, finite request ceilings and no extra
call for early success, valid empty, hard failure or rate limiting. All 28 had
complete lifecycle traces. Numbers below include failed/empty runs.

| Arm | Nonempty / 7 | Wall p50 | Wall p95/max | Total sends |
| --- | ---: | ---: | ---: | ---: |
| short | 1 | 1.154 s | 1.161 s | 8 |
| long | 2 | 1.253 s | 5.157 s | 9 |
| retry_fresh | 3 | 1.138 s | 5.310 s | 11 |
| retry_reuse | 3 | 1.154 s | 5.163 s | 11 |

The late-response fixture took 1.534 s with one longer call, versus 2.978 s
with a fresh retry and 2.823 s with client reuse. Both retries sent twice and
opened two connections. The transient fixture returned evidence in 1.566 s
(fresh) / 1.421 s (reuse), while the long call timed out empty at 5.155 s.
Each is one matched cell, not an estimated average benefit.

Base revision: `ec8024d6c970c20038c2db620663e6a0e1c7d445`. The working-tree source manifests include
all uncommitted new benchmark files, modes and hashes. Gate source-map SHA-256
(canonical JSON sorted keys): `5c51f0913d270603ee47beb19ea75ddee3e341550e2d8433e6fc52e4da1b8131`.

Benchmark probe SHA-256: `99b7ac5c972702e5895657d5ba2fa5394505573be8e334fdbe785ea0b93f67f8`. Built with
`cargo build --release --locked --features test-fixtures --example discovery_probe`.
Production gate binary: `/Users/rafaelpierre/projects/kestrel-rs-issue-79/target/release/kestrel`, `kestrel 7.0.0`,
SHA-256 `9b6e9cdb21a7eccd1dec9641b50617bb2aa08e52234909b5191178d22df34f0c`.
Dataset SHA-256: `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
Generated skill SHA-256: `2a98cf7d7cb104c4d64f51750e05ddc15fbd42f358c7ad4ffab98ad8d8ea3f18`.

## Canonical ten-question gate

**Gate NOT PASSED (8/10)**. This single complete window is an acceptance
observation; no successful rows are borrowed from other windows. Full artifacts:
`/tmp/kestrel-79-gate-v1`; each `qNN` directory contains receipts, all candidates,
provider traces, retained text and a manual judgment.
The [canonical dataset](../benchmarks/codex-search-2026-09-11/queries.json) and
[runner policy](../benchmarks/evidence-gate.md) are unchanged. Assessor: GPT-6 Codex.

Exact search flags: `--no-fetch --no-rank -k 20 --min-results 20 --search-budget 10
--output json`. Exact fetch flags: `--output json --timeout 15 --content-limit 100000
--max-response-bytes 4000000`. Two discoveries and three fetches maximum per
question; 30-second outer timeout per command. Cache and remote telemetry off.
Initial queries exactly match the manifest. All returned titles/URLs/snippets
were inspected; fetches used this run’s discovered URLs. q03 recovery was
`site:docs.python.org asyncio TaskGroup ExceptionGroup except* cancellation`;
q04 recovery was `site:postgresql.org EXPLAIN ANALYZE BUFFERS actual execution time
buffer hits`. Reasons were recorded before calls. No domain substitution.

Timings below sum actual tool processes, including recovery and failure, with
discovery and fetching separate. They exclude model deliberation and make no
percentile claim. Rust validation ran during this gate; these are not isolated
performance timings.

| ID | Judgment | Discovery s | Fetch s | Total s | Evidence / answer |
| --- | --- | ---: | ---: | ---: | --- |
| q01 | PASS | 0.726 | 0.000 | 0.726 | [Britannica snippet](https://www.britannica.com/place/Canberra): Canberra is the national capital. No fetch needed. |
| q02 | PASS | 0.729 | 0.151 | 0.880 | [Royal Observatory](https://www.rmg.co.uk/stories/space-astronomy/why-sky-blue): Rayleigh scattering grows as wavelength falls, preferentially scattering shorter blue visible light. |
| q03 | FAIL | 11.026 | 0.799 | 11.825 | [PEP 654](https://peps.python.org/pep-0654/) supports grouped exception handling; official overview fetches lacked TaskGroup sibling cancellation and cancellation distinctions. Both discoveries and all three fetches used. No complete answer. |
| q04 | FAIL | 1.178 | 1.492 | 2.670 | [Official reference](https://www.postgresql.org/docs/current/sql-explain.html), tutorial and auto_explain all failed extraction. Snippets do not establish full execution/reporting semantics. Both discoveries and all three fetches used. No complete answer; related [#157](https://github.com/rafaelpierre/kestrel-rs/issues/157). |
| q05 | PASS | 0.616 | 0.208 | 0.825 | [TraceQL guide](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): `{A} > {B}` returns B children of A; `<` reverses parent direction; `>>` includes deeper descendants. |
| q06 | PASS | 10.127 | 0.512 | 10.639 | [Requested-domain reference](https://learn.chatgpt.com/docs/config-file/config-reference): `otel.trace_exporter` accepts none/otlp-http/otlp-grpc with endpoint/protocol fields. TOML `[otel]` with `trace_exporter = "none"` is supported; HTTP table example is derived from documented nested keys. Advanced guide distinguishes log exporter. Reference truncates at 100,000 characters, after the complete tracing section. |
| q07 | PASS | 0.870 | 0.201 | 1.070 | [Official E0382](https://doc.rust-lang.org/error_codes/E0382.html): ownership moved before later use; borrow with `&` to retain ownership or clone for a separate owned value. |
| q08 | PASS | 1.042 | 0.205 | 1.247 | [NASA system summary](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): December 2025 data show no thick atmosphere evidence for b; bare rock is tentative, with stellar contamination and more observations needed. |
| q09 | PASS | 0.976 | 0.412 | 1.388 | [3.0 migration](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/) plus release notes: removed ingester/compactor/SSB, Kafka for microservices (not monolithic), vParquet4+ required, migrate flat overrides/config, parallel migration with no in-place downgrade. |
| q10 | PASS | 0.981 | 0.000 | 0.981 | [Official visitor snippet](https://www.sciencemuseum.org.uk/visit): daily 10:00–18:00, last entry 17:15; closed 24–26 December. No fetch needed. |

The q03 collection/selection gap is retained under #79 and historical gate
coordination #148; the q04 recurrence matches closed investigation #157. Neither
closed issue is silently reopened or treated as a waiver. This PR remains draft.

Validation: `cargo fmt --check`, `cargo clippy --all-targets --all-features --
-D warnings`, `cargo test --all-features` (304 passed; 10 existing ignored),
release builds, and all 66 Python benchmark tests (two existing skipped) passed.
The five focused eligibility tests and asserted 28-cell fixture repeat passed.
The current CLI installed its generated skill into the gate’s temporary project;
search/fetch help and the installed discovery/retry/budget guidance were inspected.
There is no production contract change requiring a template edit. MSRV remains
1.89; no dependencies changed. No subagents were used.

After the gate, only this documentation report was extended. Executable inputs,
benchmark scripts, dataset, generated skill and policy remained unchanged; the
PR records the final signed revision/tree alongside this frozen run identity.
