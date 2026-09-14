# Issue #206 evidence gate, historical run 1 — 2026-09-14

Historical run before rebasing onto the shared retry-loop and quorum-removal changes. See [the current run](issue-206-evidence-gate.md) for final executable acceptance.

**gate NOT PASSED (6/10).** q03, q04, q05 and q06 fail the required evidence minima. The PR remains draft. Collection failures overlap the open [Bing fidelity investigation #200](https://github.com/rafaelpierre/kestrel-rs/issues/200) and [empty-discovery study #79](https://github.com/rafaelpierre/kestrel-rs/issues/79); this run does not establish their root cause or claim a baseline comparison. No query, rubric or provider-fidelity change is included in #206.

## Provenance and fixed workflow

- Tested base revision: `deb8075335e8a1c516d42f458eeb81cb8ec7521e`; staged implementation tree: `8b1d469ff400c1521a5c16b2bcc1cf111f573bc6`.
- Binary: `/Users/rafaelpierre/projects/kestrel-rs-issue-206/target/release/kestrel`; `kestrel 8.0.0`; SHA-256 `144029dd7924fa8ba345d0583bf34c495517045bd088420d4bfebc20ae5b9d3e`.
- [Canonical q01–q10 dataset](../benchmarks/codex-search-2026-09-11/queries.json): SHA-256 `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
- Generated skill SHA-256: `8d8ade142099a5dd8ed700b0b87057fe5fa3789baf5953349f9e30ba2e4e966d`; frozen rubric SHA-256: `ec71434b776ebd53184abf2de475dff9db6f893a052888d617ee0607969097e7`.
- Assessor: Codex GPT-6, manual semantic judgments against every q01–q10 row in repository guidance.
- Full local artifacts: `/tmp/kestrel-206-gate-run1/`; consolidated report: `/tmp/kestrel206-gate-report.json`. The manifest records each source file's SHA-256 and mode. Each attempt retains exact argv, timestamps, exit code, full stdout/stderr, all candidates, provider traces, actual fetches and complete extracted content. This is the only gate run; all failures are retained.
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
| [q01](#q01) | PASS | 5.171 | 0.000 | 5.171 |
| [q02](#q02) | PASS | 5.225 | 0.188 | 5.413 |
| [q03](#q03) | FAIL | 37.078 | 0.000 | 37.078 |
| [q04](#q04) | FAIL | 63.232 | 0.000 | 63.232 |
| [q05](#q05) | FAIL | 62.984 | 0.000 | 62.984 |
| [q06](#q06) | FAIL | 10.591 | 0.350 | 10.942 |
| [q07](#q07) | PASS | 1.598 | 1.147 | 2.746 |
| [q08](#q08) | PASS | 1.263 | 0.191 | 1.454 |
| [q09](#q09) | PASS | 1.642 | 0.477 | 2.120 |
| [q10](#q10) | PASS | 2.289 | 0.294 | 2.584 |

## Validation and compatibility

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --all-features`: 334 passed, 10 existing ignored tests; no failures.
- `rustup run 1.89.0 cargo check --all-targets --all-features --locked`: passed. No dependency, Rust edition or MSRV changes.
- `cargo build --release --locked`: passed. Temporary generated-skill installation succeeded. CLI installation fixtures assert the updated typed-outcome guidance; report tests retain old strings, default discovery attempt and explicit unknown classification.
- Focused tests cover display-message independence, mutable diagnostic independence, rate-limit observations independent of recorder contents, partial deadline snapshots and never-polled cancellation. Existing local HTTP, streaming, recovery and CLI fixtures also passed.
- Logs: `/tmp/test206-target.log`, `/tmp/clippy206.log`, `/tmp/tests206.log`, `/tmp/msrv206.log`; build/install receipts reside in the gate directory.

This report is the only source file added after the frozen run. Before publication, the frozen executable inputs, dataset, generated skill, policy, file hashes/modes and binary were checked unchanged. The final signed revision therefore adds only this evidence report to the staged implementation tree above. The report-only difference does not require another live run under the repository's reuse rule. Signature, tree and remote CI status are reported in the PR.

## q01

**PASS.** The retrieved Britannica snippet explicitly identifies Canberra as the federal capital; this is text evidence, not a city URL alone.

Australia's capital is Canberra.

**Selection:** Inspected all 10 titles, URLs and snippets. Chose Britannica’s Canberra entry; state-capital lists and generic Australia entries were less direct.

**Fetch disposition:** No fetch needed for this single fact; all three fetch slots skipped.

Supporting excerpts (full text retained locally):

- [search-1](https://www.britannica.com/place/Canberra): “Canberra, federal capital of the Commonwealth of Australia.”

## q02

**PASS.** The Royal Observatory text explicitly connects Rayleigh scattering, decreasing wavelength and the blue daytime sky.

Air molecules scatter sunlight through Rayleigh scattering. Scattering becomes stronger at shorter visible wavelengths, so blue light is scattered more strongly than red light and reaches us from many directions, making the daytime sky appear blue.

**Selection:** Inspected all 10 candidates. Selected the Royal Observatory’s astronomer-reviewed explanation over general summaries and the simulator.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped once supported.

Supporting excerpts (full text retained locally):

- [fetch-1](https://www.rmg.co.uk/stories/space-astronomy/why-sky-blue): “The scattering caused by these tiny air molecules (known as Rayleigh scattering) increases as the wavelength of light decreases.”

## q03

**FAIL.** The initial nine candidates are third-party sources. The official-domain recovery returned no accepted candidates, so the required TaskGroup failure/cancellation/ExceptionGroup/except* explanation cannot be supported to the rubric.

No supported answer from official Python documentation was obtained in this run.

**Selection:** Inspected all nine initial candidates and the empty recovery output. Recovery added site:docs.python.org without changing the exception-handling intent.

**Fetch disposition:** No official URL was discovered; all three fetch slots skipped, not failed.

**Recovery query:** `site:docs.python.org Python asyncio TaskGroup exception handling ExceptionGroup`. Initial nine candidates lacked official Python documentation. Add the official documentation domain while preserving the identifiers and failure-handling intent.

## q04

**FAIL.** Both exact-manifest and bounded recovery searches returned zero accepted candidates. Provider deadline recovery did not supply the required official evidence.

No supported answer for PostgreSQL EXPLAIN ANALYZE and BUFFERS was obtained in this run.

**Selection:** Inspected both empty candidate sets and provider diagnostics. Preserved the official domain and technical identifiers in the recorded lexical refinement.

**Fetch disposition:** No source URL was discovered; all three fetch slots skipped, not failed.

**Recovery query:** `site:postgresql.org EXPLAIN ANALYZE BUFFERS documentation`. Initial attempt returned zero accepted candidates despite deadline recovery. Add documentation while preserving the domain and exact SQL identifiers.

## q05

**FAIL.** Both exact-manifest and bounded recovery searches returned zero accepted candidates. Provider deadline recovery did not supply the required official evidence.

No supported answer for TraceQL immediate parent/child versus descendant operators was obtained in this run.

**Selection:** Inspected both empty candidate sets and provider diagnostics. Preserved the official domain and technical identifiers in the recorded lexical refinement.

**Fetch disposition:** No source URL was discovered; all three fetch slots skipped, not failed.

**Recovery query:** `site:grafana.com Tempo TraceQL parent child spans structural operators`. Initial attempt returned zero accepted candidates. Add structural operators while preserving the requested domain and parent-child semantics.

## q06

**FAIL.** Both searches returned only the same CLI landing page. Its successful extraction contains neither otel nor trace_exporter, and supplies no documented configuration migration evidence. A landing page does not pass.

No supported otel/trace_exporter configuration syntax was retrieved from the requested official domain.

**Selection:** Inspected the one candidate from each search and fetched the sole distinct URL. Kept site:learn.chatgpt.com and both identifiers in recovery; no substituted domain or remembered URL was used.

**Fetch disposition:** One successful extraction but insufficient evidence; the remaining two fetches were skipped because recovery found no alternative URL.

**Recovery query:** `site:learn.chatgpt.com Codex configuration otel trace_exporter`. Initial search returned only a CLI landing page. Add configuration while preserving the required domain and configuration identifiers.

## q07

**PASS.** The official E0382 entry supplies the error explanation and repairs with ownership implications; its code remains readable.

E0382 means a variable is used after its value has moved. Assigning a non-Copy value to another owner can leave the original binding unusable. Pass a reference such as &s1 when the callee only needs to borrow it, preserving the caller’s ownership. If a separate owned value is needed, clone it; this creates a duplicate rather than transferring the original. Eligible types whose fields are Copy can implement Copy for implicit copying.

**Selection:** Inspected all 20 candidates; rejected game results, homepages, and third-party explanations in favor of the official error-code entry.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://doc.rust-lang.org/stable/error_codes/E0382.html): “A variable was used after its contents have been moved elsewhere.”
- [fetch-1](https://doc.rust-lang.org/stable/error_codes/E0382.html): “borrow the value without changing its ownership.”

## q08

**PASS.** The NASA text explicitly dates its assessment to December 2025, discusses planet b, and preserves the atmospheric uncertainty.

NASA’s December 2025 account reports that Webb had not detected signs of thick atmospheres on TRAPPIST-1 b or c. For b, the available data suggest a bare rock with no atmosphere, rather than conclusively excluding every possible atmosphere. Stellar flares and spots contaminate atmospheric signals, and further observations are needed to become more certain.

**Selection:** Inspected all 20 candidates. Selected the dated NASA system overview. Planet-e-only reports, other exoplanets, a dairy result and the 2026 Nature paper alone do not satisfy the requested entity/date; the 2025 A&A paper was a backup.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “As of December 2025”
- [fetch-1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “The current data for b suggests it may be a bare rock with no atmosphere.”

## q09

**PASS.** Both official documents explicitly cover Tempo 3.0, breaking changes and actionable migration implications.

Tempo 3.0 replaces ingesters and compactors with block-builders, live-stores, and backend scheduling/workers, and requires vParquet4 or later. Microservices deployments need Kafka and updated manifests; run 3.0 alongside 2.x, switch traffic, then decommission 2.x. Monolithic deployments need configuration and binary updates but no Kafka. SSB is removed; choose monolithic or microservices. Use tempo-cli migrate config and migrate overrides-config, remove obsolete ingester/compactor/client sections and associated alerts/panels, and convert flat overrides to scoped defaults. Legacy overrides have only a temporary opt-in. There is no in-place downgrade; routing traffic back to a still-running 2.x deployment during migration differs from downgrading. Remove mem-ballast-size-mbs, partition_ring_live_store, querier.query_live_store and ingest.enabled; keep required ingest.kafka configuration. Move per-module block settings to storage.trace.block and replace query_ingesters_until with query_backend_after. Review reduced live-store defaults, lag failures, the 30-second query cutoff and the new five-second RetryInfo hint. Update compaction flags to remove their duplicate prefix. Array != and !~ semantics change to NOT IN and MATCH NONE; review queries or temporarily use skip_optimization=true. Replace OpenCensus with OTLP, replace removed v2 CLI/SpanMetricsSummary uses, and supply timezone-aware RFC3339 or relative search timestamps.

**Selection:** Inspected all 20 candidates. Selected the official 3.0 migration and version-specific upgrade guides; excluded Grafana application release notes, older Tempo releases, generic indexes and third-party summaries.

**Fetch disposition:** Two successful, uncapped extractions; third fetch skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “Monolithic mode does not require Kafka.”
- [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/upgrade/): “Tempo 3.0 requires vParquet4 or later”
- [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/upgrade/): “There is no supported downgrade path from 3.0 to 2.x.”

## q10

**PASS.** The official visitor page supports hours, last entry, school-holiday hours and the listed exceptional dates.

London’s Science Museum normally opens daily 10:00–18:00, including school holidays, with last entry at 17:15. It closes 24–26 December, and galleries start closing 30 minutes before museum closing. The retrieved visitor page also lists a full closure on 11 November 2026 and a 17:00 closing time on 26 November 2026; separate gallery closures should be checked before visiting.

**Selection:** Inspected all 20 candidates. Selected the official visitor page over Natural History Museum results, broad London lists, and unofficial opening-hour aggregators.

**Fetch disposition:** One successful, uncapped extraction; remaining fetches skipped.

Supporting excerpts (full text retained locally):

- [fetch-1](https://www.sciencemuseum.org.uk/visit): “open daily from 10.00–18.00”
- [fetch-1](https://www.sciencemuseum.org.uk/visit): “24–26 December”
- [fetch-1](https://www.sciencemuseum.org.uk/visit): “Wednesday 11 November 2026.”
- [fetch-1](https://www.sciencemuseum.org.uk/visit): “17.00 on Thursday 26 November 2026.”
