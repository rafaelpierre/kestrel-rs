# Issue 148: retained evidence-gate observations

**Gate NOT PASSED.** Four complete windows scored **6/10, 4/10, 6/10, 4/10** respectively. W4 tests the final runner. No union of successful rows is a passing gate. [Issue #148](https://github.com/rafaelpierre/kestrel-rs/issues/148) remains open; this is a draft implementation of repeatable capture and failure diagnosis, not a claim of reliable retrieval.

## Workflow and provenance

The [canonical dataset](codex-search-2026-09-11/queries.json) and `AGENTS.md` answer minima are unchanged. See the [entry-point documentation](evidence-gate.md) and [runner policy](evidence_gate.py). Assessor: Codex GPT-6, manually inspecting every returned title, URL and snippet and the selected retained passages. Primary sources were prioritized; the rubric permits the secondary physics/astronomy evidence identified below. No other search tool or remembered URL supplied missing evidence. Every fetched URL was returned in that question/window; repeated answers were independently checked against that window’s retained text.

Commands used the exact manifest query first, then at most one documented keyword recovery. All nine default providers; metadata-only search flags `--no-fetch --no-rank -k 20 --min-results 20 --search-budget 10 --output json`. Fetch flags `--output json --timeout 15 --content-limit 100000 --max-response-bytes 4000000`. At most two searches and three direct fetches per question; 30-second outer process timeout for each. Cache disabled, upstream cache/network state uncontrolled. Remote telemetry disabled; raw provider traces and benchmark artifacts retained locally. There is no host/domain substitution for q06. Link/redirect-chain import is not supported by this runner, and no migration is inferred.

Timings below are monotonic subprocess wall seconds: **D / F / T = discovery / direct fetch / summed tool time**, including recovery and failed calls. They exclude assessor end-to-end latency. No percentiles, confidence intervals or causal performance comparison are reported. These are sequential windows within one session on one host, not independent days/regions or proof of stable coverage. Some compilation/testing overlapped early windows; speed differences must not be attributed to search changes.

- Base revision: `e7527719feff0265ea34ee7334f289f076162b76`; base Git tree: `900eba9b6c8058a0b075ea12c665b3ae0cd434f5`.
- Binary: `/Users/rafaelpierre/projects/kestrel-rs-issue-148/target/release/kestrel`; `kestrel 6.0.0`; SHA-256 `514df0ef84999eb6cf968e57d49a43a0b9797302cd86cc68c894033874258e36`.
- Dataset SHA-256: `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
- Generated skill SHA-256: `4934c1c68e4c4fd56c27f7b205f436f6756e2dd3f4604b4db62d8ffc3290d986`; rubric SHA-256: `cde7150965fee4a663da64e9bcf04207779aaa8e8e65ebeee9497a1846085a83`.
- Each initialization ran `cargo build --release --locked`, installed the generated skill into its own temporary project and saved the version/build/install receipts. All four binary, dataset, skill and rubric hashes match. Existing installed skills were not overwritten; the CLI recorded the temporary installation paths.
- Each `manifest.json` includes the complete tested source-file hash/mode map, including untracked additions. W1–W3 use the predecessor runner; W4 uses the final interruption-reservation and dataset-completeness safeguards. Network flags and semantic assessment policy stayed identical. Neither version experienced an interrupted live attempt. Earlier source maps match each other; their three added source files are retained under W1 `frozen-extra-sources/` with exact hash verification. W4 has its own matching source snapshot.

| Window | Gate | Capture runner SHA-256 | UTC discovery/fetch interval | Calls D/F |
| --- | --- | --- | --- | --- |
| W1 | NOT PASSED (6/10) | `2111639b3373ab2e4121e6043b190f650c294268660bf3f588514f95ee9fe01f` | 2026-09-13T16:50:57.122013+00:00 – 2026-09-13T16:54:35.719501+00:00 | 15/6 |
| W2 | NOT PASSED (4/10) | `2111639b3373ab2e4121e6043b190f650c294268660bf3f588514f95ee9fe01f` | 2026-09-13T16:54:46.542021+00:00 – 2026-09-13T16:58:43.691386+00:00 | 17/2 |
| W3 | NOT PASSED (6/10) | `2111639b3373ab2e4121e6043b190f650c294268660bf3f588514f95ee9fe01f` | 2026-09-13T16:59:25.819184+00:00 – 2026-09-13T17:04:27.472442+00:00 | 17/8 |
| W4 | NOT PASSED (4/10) | `cf4b59751fdcad87de55537088c1ae66a61552c27b7c787bcaf1cb2f0afabc6b` | 2026-09-13T17:05:25.419224+00:00 – 2026-09-13T17:08:38.149004+00:00 | 17/2 |

## Recovery queries

Each recovery retained the original entity, version/date and any manifest site restriction. Reasons were recorded before execution in the retained decision/request JSON.

| ID | Windows | Exact recovery query |
| --- | --- | --- |
| q03 | W1, W2, W3, W4 | `site:docs.python.org asyncio TaskGroup ExceptionGroup exception handling` |
| q04 | W1, W2, W3, W4 | `site:postgresql.org EXPLAIN ANALYZE BUFFERS documentation` |
| q05 | W2, W3, W4 | `site:grafana.com Tempo TraceQL parent child spans structural operators` |
| q06 | W2, W3, W4 | `site:learn.chatgpt.com Codex otel trace_exporter configuration` |
| q07 | W1, W2, W3, W4 | `site:doc.rust-lang.org Rust E0382 use of moved value` |
| q09 | W1, W2, W3, W4 | `site:grafana.com Grafana Tempo 3.0 release notes breaking changes` |
| q10 | W1, W2, W3, W4 | `"Science Museum" London opening hours` |

## Ten-question results

Each cell contains verdict and D/F/T seconds. The linked question section supplies the answer or abstention, selected-source passages, failure layer and retained evidence references.

| Question | W1 | W2 | W3 | W4 final |
| --- | --- | --- | --- | --- |
| [q01](#q01) | PASS 10.752/0.000/10.752 | PASS 5.980/0.000/5.980 | PASS 3.024/0.000/3.024 | PASS 10.209/0.000/10.209 |
| [q02](#q02) | PASS 2.381/2.429/4.810 | PASS 5.724/5.226/10.950 | PASS 2.482/0.353/2.835 | PASS 10.159/0.423/10.582 |
| [q03](#q03) | FAIL 14.862/3.910/18.772 | FAIL 25.700/0.000/25.700 | PASS 22.165/0.499/22.664 | FAIL 20.296/0.000/20.296 |
| [q04](#q04) | FAIL 22.343/0.000/22.343 | FAIL 25.226/0.000/25.226 | FAIL 14.129/1.377/15.507 | FAIL 20.342/0.000/20.342 |
| [q05](#q05) | PASS 1.953/1.556/3.509 | FAIL 25.197/0.000/25.197 | FAIL 22.627/0.000/22.627 | FAIL 20.345/0.000/20.345 |
| [q06](#q06) | PASS 10.849/4.179/15.028 | FAIL 24.648/0.000/24.648 | FAIL 22.026/0.000/22.026 | FAIL 20.333/0.000/20.333 |
| [q07](#q07) | FAIL 23.200/0.000/23.200 | FAIL 24.542/0.000/24.542 | PASS 21.864/0.307/22.171 | FAIL 20.345/0.000/20.345 |
| [q08](#q08) | PASS 11.320/2.556/13.876 | PASS 12.308/0.654/12.962 | PASS 11.262/0.470/11.732 | PASS 10.143/0.236/10.380 |
| [q09](#q09) | FAIL 22.928/0.000/22.928 | FAIL 25.502/0.000/25.502 | FAIL 21.658/0.000/21.658 | FAIL 20.363/0.000/20.363 |
| [q10](#q10) | PASS 22.643/0.000/22.643 | PASS 24.530/0.000/24.530 | PASS 13.180/0.000/13.180 | PASS 20.305/0.000/20.305 |

## Answers, evidence and diagnosis

Pass answers below apply only to the windows explicitly listed. Fail answers are partial/abstentions, never filled from another window. Full supporting passages and complete retained page text remain in local assessment/stdout files. Short excerpts here are deliberately limited; artifact references identify the complete retained evidence.

### q01

**W1, W2, W3, W4 PASS:** Canberra is the capital of Australia.

Britannica explicitly calls Canberra “federal capital of the Commonwealth of Australia”; Mappr states “The capital of Australia is Canberra.”

- [Selected source](https://www.britannica.com/place/Canberra): W1 `q01/search-1/stdout`, W4 `q01/search-1/stdout`.
- [Selected source](https://www.mappr.co/capital-cities/australia/): W2 `q01/search-1/stdout`, W3 `q01/search-1/stdout`.

- W1 **none**: The Britannica snippet explicitly identifies Canberra as the federal capital. Fetch disposition: All fetches skipped: explicit snippet is sufficient for this single fact.
- W2 **none**: Mappr and WorldAtlas snippets explicitly identify the capital. Fetch disposition: All fetches skipped: explicit snippet is sufficient for this single fact.
- W3 **none**: Mappr and WorldAtlas snippets explicitly identify the capital. Fetch disposition: All fetches skipped: explicit snippet is sufficient for this single fact.
- W4 **none**: The Britannica snippet explicitly identifies Canberra as the federal capital. Fetch disposition: All fetches skipped: explicit snippet is sufficient for this single fact.

### q02

**W1, W2, W3, W4 PASS:** The daytime sky is blue because air molecules Rayleigh-scatter sunlight; shorter visible blue wavelengths scatter more strongly than longer red wavelengths, approximately inversely with wavelength to the fourth power.

The physics article states “shorter (blue) wavelengths are scattered more strongly than longer (red) wavelengths.” Its surrounding text connects atmospheric Rayleigh scattering to the daytime sky.

- [Selected source](https://en.wikipedia.org/wiki/Rayleigh_scattering): W1 `q02/fetch-1/stdout`, W2 `q02/fetch-1/stdout`, W3 `q02/fetch-1/stdout`.
- [Selected source](https://en.m.wikipedia.org/wiki/Rayleigh_scattering): W4 `q02/fetch-1/stdout`.

- W1 **none**: Retrieved text connects Rayleigh scattering, atmosphere, blue sky and preferential shorter-wavelength scattering. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W2 **none**: Retrieved text connects Rayleigh scattering, atmosphere, blue sky and preferential shorter-wavelength scattering. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W3 **none**: Retrieved text connects Rayleigh scattering, atmosphere, blue sky and preferential shorter-wavelength scattering. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W4 **none**: Retrieved text connects Rayleigh scattering, atmosphere, blue sky and preferential shorter-wavelength scattering. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.

### q03

**W1 FAIL:** Partial answer: ExceptionGroup groups unrelated exceptions; except* matches and handles subgroups by exception type. Abstain on TaskGroup sibling cancellation and ordinary-failure versus cancellation propagation because the retrieved official evidence does not establish them.

**W2 FAIL:** Abstain: Ten initial third-party candidates, no official Python source, and empty official-domain recovery. Required TaskGroup propagation/cancellation evidence missing.

**W3 PASS:** In asyncio.TaskGroup, the first non-CancelledError failure cancels remaining sibling tasks. The context manager waits for tasks to finish, then raises their non-cancellation failures as an ExceptionGroup or BaseExceptionGroup. CancelledError is treated differently from an ordinary failure and should generally be propagated after cleanup; swallowing it can break structured concurrency. Use except* to handle matching exception types within a group; unmatched exceptions continue propagating. KeyboardInterrupt and SystemExit are special: after siblings are cancelled and awaited, the original exception is re-raised.

**W4 FAIL:** Abstain: Ten third-party initial candidates and empty official-domain recovery; official TaskGroup propagation/cancellation evidence missing.

The API states “the remaining tasks in the group are cancelled” and “those exceptions are combined in an ExceptionGroup or BaseExceptionGroup”. The tutorial describes handling “only the exceptions in the group that match a certain type”. PEP 654 alone in W1 does not establish the TaskGroup cancellation contract.

- [Selected source](https://peps.python.org/pep-0654/): W1 `q03/fetch-1/stdout`.
- [Selected source](https://docs.python.org/3/library/asyncio-task.html): W3 `q03/fetch-1/stdout`.
- [Selected source](https://docs.python.org/3/tutorial/errors.html): W3 `q03/fetch-2/stdout`.

- W1 **selection**: PEP 654 supports grouped exceptions and except* but not the required TaskGroup lifecycle/cancellation contract. Recovery to docs.python.org returned no candidates. Fetch disposition: PEP extraction succeeded but is only partial evidence. Other fetches skipped: no discovered official TaskGroup API page. Related #79 and #32; linked-page traversal is not supported by this runner.
- W2 **upstream**: Ten initial third-party candidates, no official Python source, and empty official-domain recovery. Required TaskGroup propagation/cancellation evidence missing. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W3 **none**: Official API and tutorial text jointly establish propagation, sibling cancellation, grouped exceptions, cancellation distinction and except* subgroup matching. Fetch disposition: Both selected official extractions succeeded, with readable code and needed explanatory passages. Third fetch skipped.
- W4 **upstream**: Ten third-party initial candidates and empty official-domain recovery; official TaskGroup propagation/cancellation evidence missing. Fetch disposition: All fetches skipped because no directly relevant official source was discovered. Blockers: #32 provider fidelity and #79 bounded recovery; PostgreSQL extraction follow-up #157 applies separately when discovery succeeds.

### q04

**W1 FAIL:** Abstain: no retrieved official evidence for EXPLAIN ANALYZE and BUFFERS.

**W2 FAIL:** Abstain: Both official PostgreSQL searches empty; no EXPLAIN ANALYZE/BUFFERS evidence.

**W3 FAIL:** Partial answer: EXPLAIN ANALYZE actually executes the statement and compares observed execution with the planner’s expectations; data modifications still occur. Abstain on the required BUFFERS explanation because no retained passage establishes it.

**W4 FAIL:** Abstain: Both domain-constrained searches returned zero candidates. No official EXPLAIN ANALYZE/BUFFERS evidence in this window; no direct fetch attempted.

The wiki says ANALYZE “will actually run the statement” and warns “if the statement changes data, that will also happen”. It does not explain BUFFERS.

- [Selected source](https://wiki.postgresql.org/wiki/Using_EXPLAIN): W3 `q04/fetch-3/stdout`.

- W1 **upstream**: Both domain-constrained searches returned zero candidates; initial Bing raw=10/rejected=10, six providers errored and DuckDuckGo reached its deadline. Fetch disposition: All fetches skipped because no eligible source was discovered. Provider fidelity #32 and bounded recovery #79 remain blockers; no claim that the documentation is absent.
- W2 **upstream**: Both official PostgreSQL searches empty; no EXPLAIN ANALYZE/BUFFERS evidence. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W3 **extraction**: Recovery discovered official docs, but sql-explain and using-explain both failed with no extractable page text. The final PostgreSQL wiki fetch succeeded and supports ANALYZE execution, but does not discuss BUFFERS. No claim about the underlying HTTP/body cause is possible from the current fetch diagnostics. Fetch disposition: Three actual fetches: two failed extractions and one successful partial-evidence page. Fetch ceiling exhausted.
- W4 **upstream**: Both domain-constrained searches returned zero candidates. No official EXPLAIN ANALYZE/BUFFERS evidence in this window; no direct fetch attempted. Fetch disposition: All fetches skipped because no directly relevant official source was discovered. Blockers: #32 provider fidelity and #79 bounded recovery; PostgreSQL extraction follow-up #157 applies separately when discovery succeeds.

### q05

**W1 PASS:** TraceQL {condA} > {condB} returns matching B spans that are direct children of A. {condA} < {condB} returns B spans that are direct parents of A. {condA} >> {condB} matches B descendants of A, including deeper generations; structural operators return the right-hand matches.

**W2, W3 FAIL:** Abstain: Both Grafana-domain searches empty; no structural-operator evidence in this window. Do not borrow the first window’s source.

**W4 FAIL:** Abstain: Both Grafana-domain searches returned zero candidates. Structural-operator evidence absent; earlier-window success cannot support this window.

Grafana states “Structural operators ALWAYS return matches from the right side of the operator.” The retained operator list distinguishes `>` direct children, `<` direct parents and `>>` descendants.

- [Selected source](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): W1 `q05/fetch-1/stdout`.

- W1 **none**: Official Grafana extraction retains operator characters, direction and direct-child versus descendant semantics; the earlier extraction gap is absent in this observation. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W2 **upstream**: Both Grafana-domain searches empty; no structural-operator evidence in this window. Do not borrow the first window’s source. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W3 **upstream**: Both Grafana-domain searches empty; no structural-operator evidence in this window. Do not borrow the first window’s source. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W4 **upstream**: Both Grafana-domain searches returned zero candidates. Structural-operator evidence absent; earlier-window success cannot support this window. Fetch disposition: All fetches skipped because no directly relevant official source was discovered. Blockers: #32 provider fidelity and #79 bounded recovery; PostgreSQL extraction follow-up #157 applies separately when discovery succeeds.

### q06

**W1 PASS:** In user-level ~/.codex/config.toml, [otel] holds telemetry settings. The trace_exporter field accepts none, otlp-http or otlp-grpc; for example [otel] / trace_exporter = "none" disables trace export. An OTLP/HTTP trace exporter uses otel.trace_exporter.<id>.endpoint, optional headers, and protocol binary or json. Log export is configured separately through exporter. Project-local otel settings are ignored according to the fetched advanced configuration page.

**W2, W3 FAIL:** Abstain: Both requested learn.chatgpt.com searches empty; no otel/trace_exporter configuration evidence in this window. Do not substitute another domain or earlier-window evidence.

**W4 FAIL:** Abstain: Both learn.chatgpt.com searches returned zero candidates. Requested otel/trace_exporter configuration evidence absent; no alternate host substituted.

The requested reference lists `otel.trace_exporter` with values `none | otlp-http | otlp-grpc` and endpoint/protocol/header fields. The advanced page shows `[otel]` TOML and user-level placement. The reference reaches the character ceiling after the relevant rows; no claim of retaining the entire remote page is made.

- [Selected source](https://learn.chatgpt.com/docs/config-file/config-advanced): W1 `q06/fetch-1/stdout`.
- [Selected source](https://learn.chatgpt.com/docs/config-file/config-reference): W1 `q06/fetch-2/stdout`.

- W1 **none**: Both requested learn.chatgpt.com configuration pages were discovered and fetched. The reference explicitly documents trace_exporter values and nested endpoint/protocol fields; advanced configuration supplies [otel] TOML syntax and user-level placement. No replacement host or migration claim is used. Fetch disposition: Both extractions succeeded. The long reference hit its character ceiling after the relevant trace_exporter rows; retained passages establish the answer, but the full remote page is not claimed retained. Third fetch skipped.
- W2 **upstream**: Both requested learn.chatgpt.com searches empty; no otel/trace_exporter configuration evidence in this window. Do not substitute another domain or earlier-window evidence. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W3 **upstream**: Both requested learn.chatgpt.com searches empty; no otel/trace_exporter configuration evidence in this window. Do not substitute another domain or earlier-window evidence. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W4 **upstream**: Both learn.chatgpt.com searches returned zero candidates. Requested otel/trace_exporter configuration evidence absent; no alternate host substituted. Fetch disposition: All fetches skipped because no directly relevant official source was discovered. Blockers: #32 provider fidelity and #79 bounded recovery; PostgreSQL extraction follow-up #157 applies separately when discovery succeeds.

### q07

**W1 FAIL:** Abstain: no official evidence for E0382 cause or repair.

**W2 FAIL:** Abstain: Initial nine candidates are eBay shopping pages; official Rust-domain recovery empty. No E0382 cause/repair evidence.

**W3 PASS:** E0382 means code tries to use a value after ownership moved elsewhere. A non-Copy assignment can move the value. When the callee only needs access, pass a reference such as &s1 so the original retains ownership; if an independent duplicate is needed and the type implements Clone, clone it before moving. Borrowing preserves ownership; cloning creates a separate value.

**W4 FAIL:** Abstain: Initial ten candidates concern fantasy books; official Rust-domain recovery empty. No E0382 cause or applicable repair evidence.

The official error page says “A variable was used after its contents have been moved elsewhere” and demonstrates borrowing “without changing its ownership”. Clone/Copy alternatives and readable examples remain in the retained text.

- [Selected source](https://doc.rust-lang.org/error_codes/E0382.html): W3 `q07/fetch-1/stdout`.

- W1 **upstream**: Initial results were unrelated forums; exact query is present in saved Bing HTML alongside those unrelated cards. Official-domain recovery was empty. Fetch disposition: All fetches skipped because no eligible source was discovered. Provider fidelity #32 and bounded recovery #79 remain blockers; no claim that the documentation is absent.
- W2 **upstream**: Initial nine candidates are eBay shopping pages; official Rust-domain recovery empty. No E0382 cause/repair evidence. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W3 **none**: The official error reference explicitly explains the move and demonstrates borrowing and cloning repairs with their ownership implications. Fetch disposition: Official error reference extraction succeeded with readable code. Remaining fetches skipped.
- W4 **upstream**: Initial ten candidates concern fantasy books; official Rust-domain recovery empty. No E0382 cause or applicable repair evidence. Fetch disposition: All fetches skipped because no directly relevant official source was discovered. Blockers: #32 provider fidelity and #79 bounded recovery; PostgreSQL extraction follow-up #157 applies separately when discovery succeeds.

### q08

**W1, W2, W3, W4 PASS:** The January 7, 2025 report describes JWST observations of TRAPPIST-1 b as consistent with either an airless surface recently renewed by geology or a thick, hazy carbon-dioxide atmosphere. The latter was considered less likely, so an atmosphere was not established. The report discusses research published December 16, 2024; it is dated 2025 context, not a claim of new observations made in 2025.

The report is dated “January 7, 2025” and discusses a “thick, hazy atmosphere of carbon dioxide”; “The latter scenario is less likely”. It reports December 2024 research in a dated 2025 context, not new observations asserted to have occurred in 2025.

- [Selected source](https://skyandtelescope.org/astronomy-news/trappist-1b-atmosphere-debated-some-stars-take-their-time-forming-planets/): W1 `q08/fetch-1/stdout`, W2 `q08/fetch-1/stdout`, W3 `q08/fetch-1/stdout`, W4 `q08/fetch-1/stdout`.

- W1 **none**: Correct planet b, explicitly dated 2025 report, both interpretations and uncertainty are retained. Planet e stories and the 2026 finding are not substituted. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W2 **none**: Correct planet b, explicitly dated 2025 report, both interpretations and uncertainty are retained. Planet e stories and the 2026 finding are not substituted. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W3 **none**: Correct planet b, explicitly dated 2025 report, both interpretations and uncertainty are retained. Planet e stories and the 2026 finding are not substituted. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.
- W4 **none**: Correct planet b, explicitly dated 2025 report, both interpretations and uncertainty are retained. Planet e stories and the 2026 finding are not substituted. Fetch disposition: Successful extraction inspected; unused fetches skipped because selected evidence supports the answer.

### q09

**W1 FAIL:** Abstain: no official Tempo 3.0 breaking-change or migration evidence.

**W2 FAIL:** Abstain: Initial ten candidates are unrelated font/software/Q&A pages; official Grafana-domain recovery empty. No Tempo 3.0 migration evidence.

**W3 FAIL:** Abstain: no official Tempo 3.0 migration evidence.

**W4 FAIL:** Abstain: Initial ten results cover general Grafana, not Tempo 3.0. Recovery returned only the Grafana homepage and Grafana download page; neither establishes Tempo 3.0 breaking changes or migration.

No window produced a directly relevant official Tempo 3.0 release/migration source. Generic Grafana pages and other products do not satisfy the requested version.

- W1 **upstream**: Initial results were Microsoft support pages despite the correct query in saved Bing HTML. Official Grafana-domain recovery was empty. Fetch disposition: All fetches skipped because no eligible source was discovered. Provider fidelity #32 and bounded recovery #79 remain blockers; no claim that the documentation is absent.
- W2 **upstream**: Initial ten candidates are unrelated font/software/Q&A pages; official Grafana-domain recovery empty. No Tempo 3.0 migration evidence. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W3 **upstream**: Initial ten candidates concern an unrelated labour-services platform; official-domain recovery is empty. Fetch disposition: All fetches skipped: no relevant official URL discovered in this question/window. Link #32 for provider fidelity and #79 for bounded recovery.
- W4 **upstream**: Initial ten results cover general Grafana, not Tempo 3.0. Recovery returned only the Grafana homepage and Grafana download page; neither establishes Tempo 3.0 breaking changes or migration. Fetch disposition: All fetches skipped because no directly relevant official source was discovered. Blockers: #32 provider fidelity and #79 bounded recovery; PostgreSQL extraction follow-up #157 applies separately when discovery succeeds.

### q10

**W1, W2, W3, W4 PASS:** The Science Museum in London opens daily 10:00–18:00, with last entry at 17:15. It is closed 24–26 December.

The official visitor snippet supplies `10.00–18.00`, closure on `24–26 December` and “Last entry is 17.15.” No unstated seasonal exception is invented.

- [Selected source](https://www.sciencemuseum.org.uk/visit): W1 `q10/search-2/stdout`, W2 `q10/search-2/stdout`, W3 `q10/search-2/stdout`, W4 `q10/search-2/stdout`.

- W1 **none**: The official visitor-page snippet gives daily hours, last entry and the holiday closure. Fetch disposition: All fetches skipped: the official snippet explicitly supplies the hours and holiday qualifications required by the rubric.
- W2 **none**: The official visitor-page snippet gives daily hours, last entry and the holiday closure. Fetch disposition: All fetches skipped: the official snippet explicitly supplies the hours and holiday qualifications required by the rubric.
- W3 **none**: The official visitor-page snippet gives daily hours, last entry and the holiday closure. Fetch disposition: All fetches skipped: the official snippet explicitly supplies the hours and holiday qualifications required by the rubric.
- W4 **none**: The official visitor-page snippet gives daily hours, last entry and the holiday closure. Fetch disposition: All fetches skipped: the official snippet explicitly supplies the hours and holiday qualifications required by the rubric.

## Follow-up scope and limitations

- [#32](https://github.com/rafaelpierre/kestrel-rs/issues/32): unrelated provider responses. W1 retained Bing HTML has the correct Rust/Tempo query in its title and unrelated result cards in the same response. Domain restrictions sometimes reject all ten raw Bing results; six provider errors and a DuckDuckGo deadline leave no recovery source. This is observed upstream evidence, not a newly implemented provider fix.
- [#79](https://github.com/rafaelpierre/kestrel-rs/issues/79): bounded recovery/source selection. Adding the exact museum phrase recovered the official visitor snippet in each window. Python/Rust/PostgreSQL official-domain recovery succeeded only in W3. No reliable coverage improvement is established.
- [#157](https://github.com/rafaelpierre/kestrel-rs/issues/157): PostgreSQL reference and usage pages both returned no extractable text in W3; the wiki supplied only partial evidence. Current standalone fetch diagnostics cannot distinguish upstream body quality from an extraction defect, so the root cause remains unproven.
- Closed [#22](https://github.com/rafaelpierre/kestrel-rs/issues/22): W1 TraceQL operators and W3 Python/Rust code were retained correctly. This observation does not reopen or duplicate its extraction-order implementation.
- There is no automatic semantic grader, arbitrary linked-page traversal, redirect-chain importer, artifact tamper-proof storage, or exhaustive provider collection guarantee. Passage validation proves a citation occurs in captured text, not that an answer is correct. Missing/unrun/partial answers stay failed. Raw content is untrusted and is never executed.

## Validation and retained artifacts

- Final runner tests: 9 passed. Complete Python benchmark suite: 53 passed against the built release executable; log `/tmp/kestrel-148-python-tests-final.log`.
- `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` passed.
- Initial `cargo test --all-features` failed the two known diagnostic phase assertions from [#124](https://github.com/rafaelpierre/kestrel-rs/issues/124): expected backoff/body, observed send. The complete rerun passed. Both logs are retained: `/tmp/kestrel-148-rust-tests.log` and `/tmp/kestrel-148-rust-tests-rerun.log`. No Rust sources, dependencies or deadlines changed.
- Release build passed; all four initializations verified the same executable. The CLI contract and generated-skill template are unchanged; installation into temporary projects verified the current generated reference without replacing user skills.
- Full artifacts: `/tmp/kestrel-148-window-1`, `/tmp/kestrel-148-window-2`, `/tmp/kestrel-148-window-3`, `/tmp/kestrel-148-window-4-final` (on macOS `/tmp` resolves to `/private/tmp`). Each contains `manifest.json`, frozen `queries.json`, `AGENTS.md`, `SKILL.md`, `report.json`, per-question `assessment.json`, and all attempt requests/receipts/stdout/stderr plus provider traces/artifacts. W1–W3 decision records are sibling JSON files; W4 decisions are inside the reserved attempt request. No actual live timeout/interruption occurred; timeout preservation is covered deterministically.
- The signed PR tree adds this sanitized report and a benchmark README pointer after W4. Verify unchanged executable inputs, dataset, skill, runner and evaluation policy against W4’s source map before publication; these documentation-only additions reuse that final run. The PR records the signed revision and complete tree verification. Raw generated traces/page bodies are not committed.
