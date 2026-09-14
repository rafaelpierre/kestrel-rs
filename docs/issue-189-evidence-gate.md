# Issue #189 evidence gate — 2026-09-14

**gate NOT PASSED (9/10).** q04 failed at extraction. The PR must remain draft. Related investigation [#157](https://github.com/rafaelpierre/kestrel-rs/issues/157) is closed; this run reproduces its documented symptom without silently reopening it. No fetched-page or relevance fix is included in #189.

## Provenance and policy

- Tested base revision: `6203cdfcadcd99619b499530417de9f341155ff4`; prepared tracked tree including the feature: `a527cd21474e12c91bf45f47905d651acac63420`.
- Binary: `/Users/rafaelpierre/projects/kestrel-rs-issue-189/target/release/kestrel`; `kestrel 6.0.5`; SHA-256 `cdc31bcf2e15ddf78543992b311859d5c003cea76fa3e28c7c296f38c32c861e`.
- [Canonical dataset](../benchmarks/codex-search-2026-09-11/queries.json): SHA-256 `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
- Frozen source-file/mode map SHA-256 (sorted compact JSON): `1271aebc301303ab021706c6c971f1f7f92b2e9580464e3b76f17391490504bd`.
- Generated installed skill SHA-256: `e17a369c7568d64e236bc1c193171cf8fa76b052937f399fd6116abc052abf16`; rubric SHA-256 `cde7150965fee4a663da64e9bcf04207779aaa8e8e65ebeee9497a1846085a83`.
- Assessor: Codex GPT-6, manual question-by-question judgments against the frozen repository rubric.
- Release build: `cargo build --release --locked` in the issue worktree. Temporary project skill installation succeeded and its generated workflow was read before retrieval. No existing installed skill was overwritten.
- Raw local artifacts: `/tmp/kestrel-189-gate-run3/` (manifest, exact argv/timestamps/exits, complete stdout/stderr, candidates, provider diagnostics, retained page text and judgments). Runner policy wrapper: `/tmp/kestrel-189-gate.py`; report: `/tmp/kestrel-189-gate-report.json`.
- Uses `benchmarks/evidence_gate.py` with the predeclared issue-specific policy below. All initial queries exactly match q01–q10; every returned title, URL and snippet was inspected.
- All default providers; no cache; remote telemetry disabled; local provider traces and benchmark artifacts enabled. Upstream cache/provider conditions are uncontrolled.
- At most two discovery calls and three direct fetches per question. Both commands have a 45s outer process timeout. Discovery uses the final 5/10/15s automatic recovery policy (32s cooperative overall allowance); fetch has a 15s timeout.

```sh
kestrel search QUERY --no-fetch --no-rank -k 20 --min-results 20 --search-budget 5 --output json
kestrel fetch DISCOVERED_URL --output json --timeout 15 --content-limit 100000 --max-response-bytes 4000000
```

Times below are summed subprocess monotonic wall seconds, including recovery and failed fetches, separated by discovery/fetch. They exclude assessor/agent latency. No live percentile or performance claim is made. All discovery calls retained candidates on their first internal attempt, so this live run did not trigger automatic discovery retry; deterministic tests cover that path.

The earlier `/tmp/kestrel-189-gate-run1/` contains build/skill provenance only and was superseded by the shared-writer correction; no searches were run. `/tmp/kestrel-189-gate-run2/` retains a failed compile during that correction; no searches were run. Run 3 is the only complete ten-question acceptance run. Nothing is unioned across runs.

This evidence document is the only file added after the frozen run. All frozen file hashes/modes and the release binary were verified unchanged before publication. The signed PR commit therefore differs from the tested prepared tree only by this report; executable inputs, dataset, generated skill and evaluation policy are unchanged.

## Individual results

| ID / evidence | Judgment | Discovery s | Fetch s | Total tool s |
| --- | --- | ---: | ---: | ---: |
| [q01](#q01) | PASS | 0.676 | 0.000 | 0.676 |
| [q02](#q02) | PASS | 0.716 | 0.130 | 0.846 |
| [q03](#q03) | PASS | 2.633 | 0.394 | 3.027 |
| [q04](#q04) | FAIL | 1.500 | 1.998 | 3.498 |
| [q05](#q05) | PASS | 0.611 | 0.183 | 0.794 |
| [q06](#q06) | PASS | 2.035 | 0.470 | 2.505 |
| [q07](#q07) | PASS | 0.780 | 0.133 | 0.913 |
| [q08](#q08) | PASS | 0.932 | 0.180 | 1.113 |
| [q09](#q09) | PASS | 0.838 | 0.474 | 1.312 |
| [q10](#q10) | PASS | 0.787 | 0.235 | 1.021 |

## Validation

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --all-features`: passed (315 tests, 10 existing ignored tests).
- `rustup run 1.89.0 cargo check --all-targets --all-features --locked`: passed. No dependency/MSRV change.
- `cargo build --release --locked`: passed; generated skill installed in a temporary project. CLI help/template regression and local HTTP/CLI fixtures passed.
- Logs: `/tmp/kestrel-189-clippy-final2.log`, `/tmp/kestrel-189-tests-final2.log`, `/tmp/kestrel-189-msrv-final2.log`, and the gate directory build/skill-install receipts.
- Controlled policy cost evidence and limits are in [discovery retries](discovery-retries.md#controlled-cost-comparison); full observed test output is `/tmp/kestrel-189-policy.log`.

## q01

**PASS — none.** The Britannica candidate explicitly calls Canberra the federal capital of Australia; this is text evidence, not inference from a URL.

Canberra is Australia's national capital.

**Selection:** Inspected all 20 candidates. Selected Britannica's Canberra entry; rejected tax, capital punishment, unrelated cities and political news. Other capital-city candidates corroborate the answer.

**Fetch disposition:** All direct fetches skipped: the short fact is explicitly supported by the retrieved snippet.

Evidence (exact short excerpts; complete text retained locally):

- [search-1: source](https://www.britannica.com/place/Canberra): “Canberra, federal capital of the Commonwealth of Australia.”

## q02

**PASS — none.** The Royal Observatory explanation identifies the mechanism and its wavelength dependence explicitly.

The sky looks blue because air molecules scatter sunlight through Rayleigh scattering. Scattering increases as visible wavelength decreases, so shorter blue wavelengths are scattered more strongly than red wavelengths and redirected across the sky.

**Selection:** Inspected all 20 candidates. Selected the Royal Observatory's explanation; excluded unrelated optical-fiber, calibration and earthquake papers. HyperPhysics was a relevant alternative.

**Fetch disposition:** One successful extraction supplied the required explanation. Remaining fetches/recovery skipped because the evidence is sufficient.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://www.rmg.co.uk/stories/space-astronomy/why-sky-blue): “The scattering caused by these tiny air molecules (known as Rayleigh scattering) increases as the wavelength of light decreases.”

## q03

**PASS — none.** Official Python API text establishes sibling cancellation, grouped exception propagation, cancellation distinctions, and a readable except-star example.

A TaskGroup awaits its tasks when the async-with context exits. An ordinary child failure (an exception other than asyncio.CancelledError) cancels sibling tasks and prevents adding tasks; after cleanup, non-cancellation failures are raised in an ExceptionGroup or BaseExceptionGroup. Handle grouped failures with an except* clause outside the async-with block, as in the documented termination example. Cancellation is distinct: CancelledError is a BaseException and should generally be re-raised after cleanup; swallowing it can disrupt structured concurrency. KeyboardInterrupt and SystemExit cancel and await siblings but are re-raised directly rather than grouped.

**Selection:** Inspected all 20 initial candidates and all 17 recovery candidates. Initial official pages were generic except for PEP 654; used one site-restricted lexical refinement to find the TaskGroup API rather than relying on third-party discussions.

**Fetch disposition:** One successful API-page extraction; no further fetch required. Code in the retained page preserves indentation.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://docs.python.org/3/library/asyncio-task.html): “the remaining tasks in the group are cancelled.”
- [fetch-1: source](https://docs.python.org/3/library/asyncio-task.html): “ExceptionGroup or BaseExceptionGroup”
- [fetch-1: source](https://docs.python.org/3/library/asyncio-task.html): “except* TerminateTaskGroup:”
- [fetch-1: source](https://docs.python.org/3/library/asyncio-task.html): “asyncio.CancelledError directly subclasses BaseException”

## q04

**FAIL — extraction.** FAIL: three official documentation fetches returned no extractable text. Two searches found relevant URLs and snippets, but those do not establish all required execution/reporting/buffer semantics. No memory-based answer substituted.

No complete answer is supported by this run. Official snippets mention EXPLAIN plan output and buffer statistics, but the required explanation of what EXPLAIN ANALYZE executes/reports could not be established from retrieved page text.

**Selection:** Inspected all 20 candidates from both searches. Selected the official EXPLAIN reference and Using EXPLAIN chapter; after both failed, searched for explicit execution/runtime/buffer terms without relaxing postgresql.org, then selected auto_explain as the remaining official explanatory alternative. Rejected mailing-list fragments and standalone ANALYZE documentation as insufficient or wrong command.

**Fetch disposition:** Three failed fetches (sql-explain, using-explain, auto-explain), all exit 1 and empty stdout; no outer timeout. Both discovery calls and all fetch slots exhausted.

Evidence (exact short excerpts; complete text retained locally):

- [search-2: source](https://www.postgresql.org/docs/current/auto-explain.html): “buffer usage statistics are printed when an execution plan is logged”

## q05

**PASS — none.** Official Grafana documentation gives operator direction, direct child/parent relationships and the descendant distinction.

TraceQL structural operators return matching spans on their right side. In {condA} > {condB}, the returned B spans are immediate children of an A span; {condA} < {condB} returns B spans that are immediate parents of A spans. Use >> for descendants at any depth rather than only immediate children; << reverses that relationship for ancestors. For example, { span.http.url = "/path/of/api" } >> { span.db.name = "db-shard-001" } selects matching database descendants of the API span.

**Selection:** Inspected all 20 candidates. Selected the structural query reference. Older release posts, landing pages and editor/API pages are secondary to the explicit operator documentation.

**Fetch disposition:** One successful extraction supplied operator definitions and examples; other calls skipped after support was established.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): “Structural operators ALWAYS return matches from the right side of the operator.”
- [fetch-1: source](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): “direct child spans of a parent matching {condA}”

## q06

**PASS — none.** Both sources were fetched at the requested official learn.chatgpt.com URLs. The trace-specific reference supports the key/values, and the advanced guide establishes TOML table syntax. The example is synthesized from those documented fields; no different domain or unsupported endpoint was substituted.

The requested learn.chatgpt.com configuration reference documents otel.trace_exporter with values none, otlp-http and otlp-grpc, plus endpoint, headers, protocol and TLS fields. Combining that key/value reference with the advanced guide's documented [otel] TOML table, a minimal trace configuration is:

[otel]
trace_exporter = "none"

This selects no trace export. The reference distinguishes trace_exporter from the log exporter field exporter; HTTP exporter protocol values are binary or json. Configuring actual export requires the corresponding exporter endpoint metadata.

**Selection:** Inspected all nine candidates. Rejected CLI/IDE/cloud landing pages; selected Configuration Reference and Advanced Configuration. The reference reached the configured character ceiling but all cited trace-exporter fields were present before truncation.

**Fetch disposition:** Two successful extractions. The advanced guide mainly illustrates log exporters; trace-specific claims come from the configuration reference. The final fetch/recovery were unnecessary.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://learn.chatgpt.com/docs/config-file/config-reference): “otel.trace_exporter none | otlp-http | otlp-grpc”
- [fetch-1: source](https://learn.chatgpt.com/docs/config-file/config-reference): “Select the OpenTelemetry trace exporter and provide any endpoint metadata.”
- [fetch-2: source](https://learn.chatgpt.com/docs/config-file/config-advanced): “[otel] / environment = "staging"”

## q07

**PASS — none.** The official Rust error page explains the move and applicable repairs with ownership implications.

E0382 means a variable is used after its value has moved elsewhere. Assigning or passing a non-Copy value can transfer ownership, leaving the old binding unusable for that value. If the callee only needs access, pass a reference such as &s1: borrowing preserves the original ownership. If independent owned data is needed and the type supports it, clone before the move; cloning creates a separate value rather than recovering the original moved value. Copy is appropriate only for types whose members support Copy.

**Selection:** Inspected all 20 candidates. Rejected the Rust game, generic language pages and third-party discussions in favor of the official E0382 entry.

**Fetch disposition:** One successful extraction including readable code; remaining calls skipped.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://doc.rust-lang.org/error_codes/E0382.html): “A variable was used after its contents have been moved elsewhere.”
- [fetch-1: source](https://doc.rust-lang.org/error_codes/E0382.html): “borrow the value without changing its ownership.”

## q08

**PASS — none.** The NASA page explicitly anchors its assessment to December 2025, discusses planet b, and states atmospheric uncertainty and observational limitations.

In its explicitly dated December 2025 assessment, NASA reports that JWST had not seen signs of thick atmospheres on TRAPPIST-1 b or c. For planet b specifically, the data suggest it may be bare rock without an atmosphere; this is a qualified interpretation, not a definitive exclusion of every possible atmosphere. NASA notes that stellar flares and spots contaminate planetary signals and that additional observations are needed for greater certainty.

**Selection:** Inspected all 20 candidates. Selected the NASA overview with explicit 2025 context. Rejected planet-e-only coverage, unrelated planets and the 2026 Nature result as sole support; the 2025 A&A modeling paper was a relevant backup.

**Fetch disposition:** One successful extraction supports the dated, qualified answer. Remaining calls skipped.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “As of December 2025”
- [fetch-1: source](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “The current data for b suggests it may be a bare rock with no atmosphere.”

## q09

**PASS — none.** Retrieved both official migration and version-specific upgrade documentation, with actionable changes and the mode-specific Kafka distinction. The two pages differ on downgrade wording, so the answer preserves that limitation.

Tempo 3.0 replaces ingesters/compactors with block-builders, live-stores and backend scheduling/workers, requires vParquet4+, and removes scalable single-binary mode. Choose monolithic mode (no Kafka) or provision Kafka and new components for microservices; use tempo-cli migrate config and update deployment manifests. Remove obsolete ingester/compactor/client blocks and local_blocks processing. Migrate flat overrides to scoped defaults, with legacy opt-in only temporary. During parallel migration, disable 3.0 compaction until 2.x compactors are stopped and validate historical queries against shared storage.

The upgrade reference additionally requires removing mem-ballast-size-mbs, partition_ring_live_store, querier.query_live_store and ingest.enabled; centralize block settings under storage.trace.block. Replace query_ingesters_until with query_backend_after and update duplicated compaction flag prefixes. Review reduced live-store/query defaults, default lag errors and the 30-second query cutoff. RetryInfo now defaults to five seconds. Array !=/!~ semantics change; review queries or use skip_optimization=true. Migrate OpenCensus receivers to OTLP; removed v2 CLI commands/SpanMetricsSummary need replacement, and CLI search timestamps require timezone-aware RFC3339 or relative time. The upgrade reference says no supported downgrade path, even though the migration guide discusses operational rollback; do not promise downgrade safety.

**Selection:** Inspected all 20 candidates. Selected the 3.0 migration and upgrade guides; rejected Grafana application releases, old Tempo releases, generic release indexes and third-party summaries.

**Fetch disposition:** Two successful complete extractions. No third fetch needed.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “Ingesters, the scalable single binary (SSB) mode, and the compactor target are removed.”
- [fetch-2: source](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/upgrade/): “Tempo 3.0 requires vParquet4 or later”
- [fetch-2: source](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/upgrade/): “No downgrade path: There is no supported downgrade path from 3.0 to 2.x.”

## q10

**PASS — none.** The official visitor page supports normal hours, last entry, school-holiday hours and listed date-specific exceptions.

London's Science Museum normally opens daily 10:00–18:00, including school holidays, with last entry at 17:15; galleries begin closing 30 minutes before the museum. It is closed 24–26 December. The retrieved visitor page also lists a full museum closure on 11 November 2026 and a 17:00 closing time on 26 November 2026. Individual gallery closures are separately listed; check those when planning a visit.

**Selection:** Inspected all 20 candidates. Selected the Science Museum visitor page; rejected Natural History Museum pages, broad London lists and unofficial opening-hours aggregators.

**Fetch disposition:** One successful extraction. No remaining calls needed.

Evidence (exact short excerpts; complete text retained locally):

- [fetch-1: source](https://www.sciencemuseum.org.uk/visit): “open daily from 10.00–18.00”
- [fetch-1: source](https://www.sciencemuseum.org.uk/visit): “24–26 December”
- [fetch-1: source](https://www.sciencemuseum.org.uk/visit): “Wednesday 11 November 2026.”
- [fetch-1: source](https://www.sciencemuseum.org.uk/visit): “17.00 on Thursday 26 November 2026.”
