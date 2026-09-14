# Issue 23 validation — 2026-09-14

## Release latency

Same deterministic benchmark and release compiler (rustc 1.96.0, Apple Silicon macOS), baseline ae4d99c versus optimized tested tree d91f7a29d10db4163594d291306026bc10ff420f. Two warmups and eleven samples per condition per run; baseline/optimized/optimized/baseline order. Entries below average the two per-run medians (nearest-rank percentile convention). No builds/tests overlapped timing; live gate discovery did overlap, so scheduling remains uncontrolled. Preliminary measurements are retained separately and excluded from this matched comparison. These measure ranking/filtering plus result destruction, excluding input cloning, not end-to-end search. See [reproduction](ranking-statistics.md).

| Tokens/field | Candidates | Policy | Before ms | After ms | Speedup |
|---|---|---|---|---|---|
| 200 | 15 | snippet | 0.262 | 0.191 | 1.37x |
| 200 | 15 | hybrid | 0.447 | 0.366 | 1.22x |
| 200 | 15 | rrf | 0.260 | 0.003 | 75.24x |
| 200 | 15 | filter | 0.325 | 0.254 | 1.28x |
| 200 | 100 | snippet | 4.066 | 1.280 | 3.18x |
| 200 | 100 | hybrid | 5.307 | 2.462 | 2.16x |
| 200 | 100 | rrf | 4.026 | 0.017 | 244.00x |
| 200 | 100 | filter | 4.426 | 1.698 | 2.61x |
| 200 | 200 | snippet | 13.665 | 2.538 | 5.38x |
| 200 | 200 | hybrid | 16.182 | 4.960 | 3.26x |
| 200 | 200 | rrf | 13.579 | 0.032 | 425.16x |
| 200 | 200 | filter | 14.322 | 3.376 | 4.24x |
| 200 | 400 | snippet | 47.930 | 5.053 | 9.49x |
| 200 | 400 | hybrid | 52.728 | 9.861 | 5.35x |
| 200 | 400 | rrf | 47.793 | 0.062 | 773.71x |
| 200 | 400 | filter | 49.153 | 6.741 | 7.29x |
| 2000 | 15 | snippet | 1.887 | 1.704 | 1.11x |
| 2000 | 15 | hybrid | 3.547 | 3.395 | 1.04x |
| 2000 | 15 | rrf | 1.820 | 0.004 | 507.92x |
| 2000 | 15 | filter | 2.474 | 2.323 | 1.06x |
| 2000 | 100 | snippet | 14.870 | 11.592 | 1.28x |
| 2000 | 100 | hybrid | 26.581 | 22.537 | 1.18x |
| 2000 | 100 | rrf | 14.740 | 0.018 | 813.25x |
| 2000 | 100 | filter | 18.922 | 15.915 | 1.19x |
| 2000 | 200 | snippet | 35.576 | 23.538 | 1.51x |
| 2000 | 200 | hybrid | 58.675 | 45.572 | 1.29x |
| 2000 | 200 | rrf | 35.138 | 0.037 | 962.15x |
| 2000 | 200 | filter | 43.776 | 31.879 | 1.37x |
| 2000 | 400 | snippet | 91.297 | 46.226 | 1.98x |
| 2000 | 400 | hybrid | 135.922 | 90.888 | 1.50x |
| 2000 | 400 | rrf | 91.037 | 0.067 | 1349.52x |
| 2000 | 400 | filter | 106.011 | 62.691 | 1.69x |

All matched mean medians decreased. Small differences at low candidate counts are within plausible scheduling variance; the large-pool improvement and approximately linear optimized scaling are the stronger observations. No statistically reliable population p95 or live-network speedup is claimed.

Benchmark executable SHA-256:
- `baseline`: `1c2bbc84f354b33593bc690b9f6cbce198c68aa49849adf338c5e1df409677be`
- `optimized-final`: `89f1c477ac827f996e90c9f9adb17efeb60b78752a8a9bfb15bf73d81467d5d4`

## Correctness and compatibility

Passed cargo fmt --check, cargo clippy --all-targets --all-features -- -D warnings, cargo test --all-features (279 passed, 10 ignored), targeted ranking tests (17 passed), and cargo build --release --locked. The test suite includes exact score-bit reference equivalence, signed empty sums, duplicate terms, threshold boundaries, and RRF provenance/ties/query fairness. Existing generated-skill parsing/schema checks and the added ranking-guidance check pass. The final executable installed its generated skill into a temporary project; search/fetch help was retained alongside it. No dependencies, CLI flags, public schemas or MSRV changed.

## Canonical evidence gate

**Gate NOT PASSED (9/10)**. User requested proceeding with the PR without further PostgreSQL investigation; PR remains draft. Related historical investigation: [#157](https://github.com/rafaelpierre/kestrel-rs/issues/157).

Dataset: [q01–q10](codex-search-2026-09-11/queries.json). One complete acceptance window, not a multi-window reliability study. Assessor: GPT-6 Codex, manual AGENTS.md per-question minima. Exact manifest query first; recoveries preserved intent and required domains/versions/dates. At most two metadata discoveries and three selected direct fetches per question. All default providers, --no-fetch --no-rank -k 20 --min-results 20 --search-budget 10 --output json; fetch --output json --timeout 15 --content-limit 100000 --max-response-bytes 4000000. Outer command timeout 30 seconds. Cache and remote telemetry disabled; local diagnostics retained. No policy changes, alternate search tools or remembered URLs. Timings below sum subprocess wall seconds including failed attempts/recovery, separate from agent latency; no percentile comparison with fetched search.

Tested base revision `ae4d99ca682132336c7293cf6afd2f685529cf58` plus tracked diff/tree `d91f7a29d10db4163594d291306026bc10ff420f`. Binary `/Users/rafaelpierre/projects/kestrel-rs-issue-23/target/release/kestrel`, version `kestrel 6.0.4`, SHA-256 `577fc2158c5a89cf9708d68d6671e54a4f4bf97611bba132eaec12c04703388d`. Dataset SHA-256 `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`. Skill SHA-256 `5f978955dfee38a4e02f71e5acf734c191ba3c75dd50c6e0f71cb5520933b56a`. The only post-run source addition is this validation report; executable inputs, dataset and policy are unchanged. Final signed revision is recorded in the PR.

| ID | Judgment | Search s | Fetch s | Total s | Evidence |
|---|---|---|---|---|---|
| q01 | PASS | 2.694 | 0.000 | 2.694 | [source 1](https://www.australia.com/en-us/places/australian-capital-territory.html) |
| q02 | PASS | 10.142 | 0.000 | 10.142 | [source 1](http://hyperphysics.phy-astr.gsu.edu/hbase/atmos/blusky.html), [source 2](https://www.rmg.co.uk/stories/space-astronomy/why-sky-blue) |
| q03 | PASS | 7.193 | 2.262 | 9.455 | [source 1](https://docs.python.org/3/library/asyncio-task.html) |
| q04 | FAIL | 4.947 | 8.012 | 12.959 | No sufficient evidence; three empty extractions |
| q05 | PASS | 2.514 | 5.242 | 7.756 | [source 1](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/) |
| q06 | PASS | 3.577 | 2.770 | 6.346 | [source 1](https://learn.chatgpt.com/docs/config-file/config-sample) |
| q07 | PASS | 6.341 | 10.956 | 17.297 | [source 1](https://doc.rust-lang.org/error_codes/E0382.html) |
| q08 | PASS | 13.691 | 13.491 | 27.182 | [source 1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/) |
| q09 | PASS | 13.746 | 25.582 | 39.327 | [source 1](https://grafana.com/docs/tempo/latest/release-notes/v3-0/) |
| q10 | PASS | 7.560 | 10.825 | 18.385 | [source 1](https://www.londoncitybreak.com/science-museum) |

### Answers and evidence assessment

- **q01**: Australia's capital is Canberra.
- **q02**: The sky is blue because atmospheric molecules scatter sunlight through Rayleigh scattering. Shorter visible wavelengths, including blue, scatter more strongly than longer red wavelengths.
- **q03**: In an asyncio TaskGroup, the first ordinary task failure cancels remaining sibling tasks; the group waits for them, then raises their non-cancellation exceptions in an ExceptionGroup or BaseExceptionGroup. CancelledError does not itself trigger that ordinary-failure rule and should generally be re-raised after cleanup. The documentation demonstrates except* TerminateTaskGroup to handle the matching exception type from the group. KeyboardInterrupt and SystemExit are re-raised specially after siblings finish.
- **q04**: No supported complete answer: official EXPLAIN and Using EXPLAIN pages were discovered, but all three direct fetches returned no extractable text. Snippets do not establish execution semantics.
- **q05**: TraceQL {A} > {B} returns B spans that are immediate children of A; {A} < {B} returns B spans that are immediate parents of A. The >> operator instead matches descendants at any depth. Standard structural operators return matching right-hand spans.
- **q06**: The requested learn.chatgpt.com sample documents [otel] with trace_exporter = "none" by default and supports none, otlp-http and otlp-grpc. Its OTLP/gRPC example uses [otel.trace_exporter."otlp-grpc"], endpoint = "https://otel.example.com:4317", and headers = { "x-otlp-meta" = "abc123" }. Use the exporter table in place of the scalar none setting.
- **q07**: E0382 means a variable is used after ownership of its non-Copy value moved elsewhere. Borrowing with & lets another function use it without transferring ownership; clone() creates a duplicate when independent ownership is needed. Copy is appropriate only for eligible types whose members are Copy.
- **q08**: NASA explicitly describes the evidence as of December 2025: Webb had not seen signs of a thick atmosphere on TRAPPIST-1 b, and the data suggested it may be bare rock without an atmosphere. This is not certainty that every atmosphere is absent. Stellar activity complicates interpretation and follow-up observations are needed.
- **q09**: Tempo 3.0 replaces ingesters with the new read/write architecture; microservices mode requires Kafka-compatible buffering while monolithic mode does not. Remove ingester, ingester_client, compactor and metrics_generator_client configuration and obsolete alerts; migrate with tempo-cli migrate config. Compaction moves to backend scheduler/worker; v2 encoding and its CLI tools are removed. Move module block settings to storage.trace.block, replace OpenCensus with OTLP, and use GOMEMLIMIT instead of mem-ballast-size-mbs. Convert flat overrides with migrate overrides-config. Audit stricter filter/regex validation and array != (NOT IN) / !~ (MATCH NONE) semantics. Use timezone-qualified RFC3339 timestamps; scalable-single-binary and 32-bit ARM archives are removed. Review new retry, high-lag error and 30-second query cutoff defaults.
- **q10**: The retrieved London Science Museum visitor listing states daily opening hours of 10:00–18:00, closed 24, 25 and 26 December. This is a secondary visitor source; official-domain discovery returned shops and blogs rather than visitor hours.

Full argv, timestamps, exit codes, all candidates/snippets, diagnostics, retained page text, selected passages and manual judgments: `/tmp/kestrel-issue23-gate-1`; machine report: `/tmp/issue23-gate-report.json`. Full benchmark samples, preliminary runs, executable copies and receipts: `/tmp/kestrel-issue23-latency`. Raw third-party pages and traces remain local. Failed and successful attempts are preserved.
