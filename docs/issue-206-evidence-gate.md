# Issue #206 evidence gate — 2026-09-14

**gate NOT PASSED (8/10).** q04 and q07 fail the required evidence minima. The PR remains draft. Collection failures overlap the open [Bing fidelity investigation #200](https://github.com/rafaelpierre/kestrel-rs/issues/200) and [empty-discovery study #79](https://github.com/rafaelpierre/kestrel-rs/issues/79); this run does not establish their root cause or claim a baseline comparison. No query, rubric or provider-fidelity change is included in #206.

## Provenance and fixed workflow

- Tested base revision: `e504858c3e594983537a693dbfe5a3b09bf2ff1e`; staged implementation tree: `e64df1042eb3a739f9074b384a023483b30d68f7`.
- Binary: `/Users/rafaelpierre/projects/kestrel-rs-issue-206/target/release/kestrel`; `kestrel 9.0.0`; SHA-256 `ddbb72383f88afcf1c7dee11f93c0ab9f3aa0c7976d3c1991465923287c3c543`.
- [Canonical q01–q10 dataset](../benchmarks/codex-search-2026-09-11/queries.json): SHA-256 `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
- Generated skill SHA-256: `6a839db46271402dc74fbe0a1a11fe359ab4e2760be27709ac26746cd51d19d3`; frozen rubric SHA-256: `ec71434b776ebd53184abf2de475dff9db6f893a052888d617ee0607969097e7`.
- Assessor: Codex GPT-6, manual semantic judgments against every q01–q10 row in repository guidance.
- Full local artifacts: `/tmp/kestrel-206-gate-run2/`; consolidated report: `/tmp/kestrel206-gate-report-run2.json`. The manifest records each source file's SHA-256 and mode. Each attempt retains exact argv, timestamps, exit code, full stdout/stderr, all candidates, provider traces, actual fetches and complete extracted content. This is the complete final run after rebasing onto `e504858` (shared HTTP retries and quorum removal). The earlier [run 1](issue-206-evidence-gate-run1.md), which passed 6/10, is retained separately; results are not combined.
- Runner: `benchmarks/evidence_gate.py` through `/tmp/kestrel206_gate.py`, SHA-256 `1834ff5d17047f029bea1a85506a3f43738e40e6ec5bc67a3008f6896f09229a`. The wrapper fixes initial discovery budget to 5 seconds and the outer process timeout to 45 seconds before initialization. It leaves the canonical queries/rubric unchanged.
- At most two discovery calls and three direct fetches per question; first call exactly matches the manifest. Default provider order; no cache/recovery flags; upstream caches and provider availability uncontrolled. Remote telemetry disabled; local traces and benchmark capture enabled.
- Search uses the existing 5/10/15-second internal recovery budgets and 32-second cooperative allowance. Fetch timeout is 15 seconds. Both commands have a 45-second outer process limit. URLs came only from this question's current-run candidates; no remembered URLs, external search tools or answer keys supplied missing evidence.
- The release binary was built in this worktree. Its generated skill was installed into a temporary project, inspected alongside search/fetch help, and used for discover–inspect–read selection. Existing installed skills were not overwritten.

```sh
kestrel search QUERY --no-fetch --no-rank -k 20 --min-results 20 --search-budget 5 --output json
kestrel fetch DISCOVERED_URL --output json --timeout 15 --content-limit 100000 --max-response-bytes 4000000
```

Timing is summed subprocess monotonic wall seconds by discovery/fetch, including recovery, cancellations and failed/empty calls. It excludes assessor/agent end-to-end latency. All searches were metadata-only, all reported fetches were direct fetches. No percentile is calculated and no performance claim is made from this single live observation.

## Individual results

| ID / evidence | Judgment | Discovery s | Fetch s | Total tool s |
| --- | --- | ---: | ---: | ---: |
| [q01](#q01) | PASS | 5.182 | 0.000 | 5.182 |
| [q02](#q02) | PASS | 5.148 | 0.188 | 5.336 |
| [q03](#q03) | PASS | 6.064 | 0.368 | 6.432 |
| [q04](#q04) | FAIL | 31.809 | 0.000 | 31.809 |
| [q05](#q05) | PASS | 0.721 | 0.239 | 0.960 |
| [q06](#q06) | PASS | 5.140 | 0.418 | 5.559 |
| [q07](#q07) | FAIL | 36.208 | 0.000 | 36.208 |
| [q08](#q08) | PASS | 5.190 | 0.184 | 5.374 |
| [q09](#q09) | PASS | 5.186 | 0.481 | 5.667 |
| [q10](#q10) | PASS | 5.157 | 0.291 | 5.448 |

## Validation and compatibility

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --all-features`: 340 passed, 10 existing ignored tests; no failures.
- `rustup run 1.89.0 cargo check --all-targets --all-features --locked`: passed. No dependency, Rust edition or MSRV changes.
- `cargo build --release --locked`: passed. Temporary generated-skill installation succeeded. CLI installation fixtures assert the updated typed-outcome guidance; report tests retain old strings, default discovery attempt and explicit unknown classification.
- Focused tests cover display-message independence, mutable diagnostic independence, rate-limit observations independent of recorder contents, partial deadline snapshots and never-polled cancellation. Existing local HTTP, streaming, recovery and CLI fixtures also passed.
- Logs: `/tmp/test206-target.log`, `/tmp/clippy206-run2.log`, `/tmp/tests206-run2.log`, `/tmp/msrv206-run2.log`; build/install receipts reside in the gate directory.

This report is the only source file added after the frozen run. Before publication, the frozen executable inputs, dataset, generated skill, policy, file hashes/modes and binary were checked unchanged. The final signed revision therefore adds only this evidence report to the staged implementation tree above. The report-only difference does not require another live run under the repository's reuse rule. Signature, tree and remote CI status are reported in the PR.

## q01

**PASS.** The retrieved Britannica snippet explicitly identifies Canberra as the federal capital; this is text evidence, not a city URL alone.

Australia's capital is Canberra.

**Selection:** Inspected all 10 current-run titles, URLs and snippets (identical to run 1); the Britannica Canberra snippet directly states the capital.

**Fetch disposition:** No fetch needed for this single fact; all three fetch slots skipped.

Supporting excerpts (full text retained locally):

- [search-1](https://www.britannica.com/place/Canberra): “Canberra, federal capital of the Commonwealth of Australia.”

## q02

**PASS.** The Royal Observatory text explicitly connects Rayleigh scattering, decreasing wavelength and the blue daytime sky.

Air molecules scatter sunlight through Rayleigh scattering. Scattering becomes stronger at shorter visible wavelengths, so blue light is scattered more strongly than red light and reaches us from many directions, making the daytime sky appear blue.

**Selection:** Inspected all 10 current-run candidates; selected the Royal Observatory over general summaries, Wikipedia and a simulator. Read the newly fetched wavelength explanation.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped once supported.

Supporting excerpts (full text retained locally):

- [fetch-1](https://www.rmg.co.uk/stories/space-astronomy/why-sky-blue): “The scattering caused by these tiny air molecules (known as Rayleigh scattering) increases as the wavelength of light decreases.”

## q03

**PASS.** Official TaskGroup documentation covers ordinary failure propagation, sibling cancellation and grouped exceptions; the official tutorial explains except* subgroup selection and propagation.

When a TaskGroup child raises an exception other than asyncio.CancelledError, the group cancels its remaining tasks and waits for them. It then raises their non-cancellation failures together as an ExceptionGroup or BaseExceptionGroup. KeyboardInterrupt and SystemExit are special: siblings are still cancelled and awaited, but the original exception is re-raised. Internal cancellation can interrupt the enclosing async-with body without escaping that block; external cancellation remains distinct. Coroutines should generally propagate CancelledError after cleanup rather than swallow it. Use except* ValueError as errors, for example, to handle matching members of a raised exception group; unmatched members continue to propagate. The official examples preserve readable indentation.

**Selection:** Inspected 20 initial and 19 recovery candidates. Initial PEP/general documentation was less direct; recovery supplied the TaskGroup reference and exception tutorial.

**Fetch disposition:** Two successful uncapped extractions; third fetch skipped.

**Recovery query:** `site:docs.python.org Python asyncio TaskGroup exception handling ExceptionGroup`. Initial candidates include PEP 654 and generic Python docs but lack the specific TaskGroup failure-handling reference; target the official documentation domain.

Supporting excerpts (full text retained locally):

- [fetch-1](https://docs.python.org/3/library/asyncio-task.html): “the remaining tasks in the group are cancelled.”
- [fetch-1](https://docs.python.org/3/library/asyncio-task.html): “those exceptions are combined in an ExceptionGroup or BaseExceptionGroup”
- [fetch-2](https://docs.python.org/3/tutorial/errors.html): “each except* clause extracts from the group exceptions of a certain type”

## q04

**FAIL.** The initial 20 candidates are mailing-list/patch discussions, not the requested command documentation. Recovery returned no accepted candidates. Snippets do not establish a complete explanation of execution and buffer reporting.

The run did not retrieve the official command documentation needed to explain EXPLAIN ANALYZE and BUFFERS to the rubric.

**Selection:** Inspected all 20 initial titles, URLs and snippets and the empty recovery result. Added documentation while preserving the domain and SQL identifiers.

**Fetch disposition:** No suitable documentation URL discovered; all three fetch slots skipped, not failed.

**Recovery query:** `site:postgresql.org EXPLAIN ANALYZE BUFFERS documentation`. Initial 20 results are mailing-list and patch discussions; add documentation to seek the official command reference while preserving domain and SQL terms.

## q05

**PASS.** The official construction reference states operator direction, direct-child/direct-parent relationships and descendants, with a query example.

TraceQL structural operators return matching spans on their right-hand side. {condA} > {condB} selects B spans that are immediate children of an A parent; {condA} < {condB} selects B spans that are immediate parents of an A child. By contrast, {condA} >> {condB} selects descendants, potentially more than one level below A. For example, { span.http.url = "/path/of/api" } >> { span.db.name = "db-shard-001" } finds matching database descendants of the API span; use > for immediate children only.

**Selection:** Inspected all 20 current candidates; selected the construction reference over general architecture, data-source, release and metrics pages.

**Fetch disposition:** One successful uncapped extraction; remaining fetches skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): “Structural operators ALWAYS return matches from the right side of the operator.”
- [fetch-1](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): “direct child spans of a parent matching {condA}”

## q06

**PASS.** Two current-run pages on the requested official domain provide the trace-exporter field schema and supported TOML conventions. This is configuration evidence, not a CLI landing page or substituted domain.

The requested learn.chatgpt.com configuration reference documents otel.trace_exporter choices none, otlp-http and otlp-grpc, with endpoint, headers, HTTP protocol (binary/json), and TLS path fields. A supported disabled-tracing configuration is:

```toml
[otel]
trace_exporter = "none"
```

For OTLP/HTTP, the documented nested keys can be expressed as:

```toml
[otel.trace_exporter.otlp-http]
endpoint = "https://otel.example.com/v1/traces"
protocol = "binary"
```

The endpoint is an illustrative placeholder. The advanced configuration page demonstrates the [otel] section and log exporter configuration; exporter controls event/log export and must not be mistaken for trace_exporter.

**Selection:** Inspected all 8 current candidates. Selected configuration reference and advanced configuration; rejected the CLI/IDE/cloud/security overviews.

**Fetch disposition:** Two successful uncapped extractions; third fetch skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://learn.chatgpt.com/docs/config-file/config-reference): “otel.trace_exporter	none | otlp-http | otlp-grpc”
- [fetch-1](https://learn.chatgpt.com/docs/config-file/config-reference): “otel.trace_exporter.<id>.protocol	binary | json”
- [fetch-2](https://learn.chatgpt.com/docs/config-file/config-advanced): “[otel]
environment = "staging"”

## q07

**FAIL.** Initial results are game pages, generic language pages or repositories. The official-domain recovery returned zero candidates; no official error-specific text supports the required explanation and repair.

No official E0382 explanation and applicable ownership repair was retrieved in this run.

**Selection:** Inspected all 10 initial candidates and the empty recovery. Targeted doc.rust-lang.org while preserving Rust, E0382 and use of moved value.

**Fetch disposition:** No suitable error-documentation URL discovered; all three fetch slots skipped, not failed.

**Recovery query:** `site:doc.rust-lang.org Rust E0382 use of moved value`. Initial ten candidates are game pages or generic Rust material. Target the official documentation domain while preserving the exact error identifier and ownership intent.

## q08

**PASS.** The NASA text explicitly dates its assessment to December 2025, discusses planet b, and preserves the atmospheric uncertainty.

NASA’s December 2025 account reports that Webb had not detected signs of thick atmospheres on TRAPPIST-1 b or c. For b, the available data suggest a bare rock with no atmosphere, rather than conclusively excluding every possible atmosphere. Stellar flares and spots contaminate atmospheric signals, and further observations are needed to become more certain.

**Selection:** Inspected all 10 current-run candidates. Selected NASA’s explicitly December-2025 assessment of b; rejected e-focused coverage and other-year reports as sole evidence. Read the newly fetched planetary finding and uncertainty.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “As of December 2025”
- [fetch-1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “The current data for b suggests it may be a bare rock with no atmosphere.”

## q09

**PASS.** Both official documents explicitly cover Tempo 3.0, breaking changes and actionable migration implications.

Tempo 3.0 replaces ingesters and compactors with block-builders, live-stores, and backend scheduling/workers, and requires vParquet4 or later. Microservices deployments need Kafka and updated manifests; run 3.0 alongside 2.x, switch traffic, then decommission 2.x. Monolithic deployments need configuration and binary updates but no Kafka. SSB is removed; choose monolithic or microservices. Use tempo-cli migrate config and migrate overrides-config, remove obsolete ingester/compactor/client sections and associated alerts/panels, and convert flat overrides to scoped defaults. Legacy overrides have only a temporary opt-in. There is no in-place downgrade; routing traffic back to a still-running 2.x deployment during migration differs from downgrading. Remove mem-ballast-size-mbs, partition_ring_live_store, querier.query_live_store and ingest.enabled; keep required ingest.kafka configuration. Move per-module block settings to storage.trace.block and replace query_ingesters_until with query_backend_after. Review reduced live-store defaults, lag failures, the 30-second query cutoff and the new five-second RetryInfo hint. Update compaction flags to remove their duplicate prefix. Array != and !~ semantics change to NOT IN and MATCH NONE; review queries or temporarily use skip_optimization=true. Replace OpenCensus with OTLP, replace removed v2 CLI/SpanMetricsSummary uses, and supply timezone-aware RFC3339 or relative search timestamps.

**Selection:** Inspected all 10 current-run candidates. Selected the official migration and upgrade guides over release indexes, older releases and a release announcement. Read the newly fetched 3.0 changes.

**Fetch disposition:** Two successful, uncapped extractions; third fetch skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “Monolithic mode does not require Kafka.”
- [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/upgrade/): “Tempo 3.0 requires vParquet4 or later”
- [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/upgrade/): “There is no supported downgrade path from 3.0 to 2.x.”

## q10

**PASS.** The official visitor page supports hours, last entry, school-holiday hours and the listed exceptional dates.

London’s Science Museum normally opens daily 10:00–18:00, including school holidays, with last entry at 17:15. It closes 24–26 December, and galleries start closing 30 minutes before museum closing. The retrieved visitor page also lists a full closure on 11 November 2026 and a 17:00 closing time on 26 November 2026; separate gallery closures should be checked before visiting.

**Selection:** Inspected all 10 current-run candidates. Selected the official visitor page over other museums and unofficial aggregators. Read the newly fetched hours and exceptions.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://www.sciencemuseum.org.uk/visit): “open daily from 10.00–18.00”
- [fetch-1](https://www.sciencemuseum.org.uk/visit): “24–26 December”
- [fetch-1](https://www.sciencemuseum.org.uk/visit): “Wednesday 11 November 2026.”
- [fetch-1](https://www.sciencemuseum.org.uk/visit): “17.00 on Thursday 26 November 2026.”
