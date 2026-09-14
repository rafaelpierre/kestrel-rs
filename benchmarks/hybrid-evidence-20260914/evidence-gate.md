# Ten-question evidence gate — issue #78

**Gate NOT PASSED (9/10).** q04 remains unsupported; the PR must stay draft.
Dataset: [canonical q01–q10](../codex-search-2026-09-11/queries.json).
All first searches used the exact manifest FTS query. The current generated skill
was installed into a temporary project and read before discovery. Assessor:
Codex GPT-6, unblinded, using the individual AGENTS.md minima.

Full artifacts: `/tmp/kestrel-78-gate-v3/`; report `/tmp/78-gate-report.json`.
The earlier `/tmp/kestrel-78-gate/` initialization failed on source drift and is
retained. `-v2/` completed initial searches but was invalidated by the study-harness
capture correction; its unassessed questions are NOT passes. Only v3 is graded.
No successful rows are combined across runs.

## Frozen execution

* Base revision: `ec8024d6c970c20038c2db620663e6a0e1c7d445`; base tree `4b599572dc1ab308b21a60fcfec21ddd7329cc11`.
* Binary: `/Users/rafaelpierre/projects/kestrel-rs-issue-78/target/release/kestrel` (`kestrel 7.0.0`).
* Binary SHA-256: `9b6e9cdb21a7eccd1dec9641b50617bb2aa08e52234909b5191178d22df34f0c`.
* Dataset SHA-256: `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.
* Generated skill SHA-256: `2a98cf7d7cb104c4d64f51750e05ddc15fbd42f358c7ad4ffab98ad8d8ea3f18`.
* Complete source/mode snapshot SHA-256: `fcbf7fcf27643531d643b49df96923b1b96bd2ea8567be2dff33fa3bc89dadfa`; individual hashes remain in `manifest.json`.

Discovery: `search QUERY --no-fetch --no-rank -k 20 --min-results 20
--search-budget 10 --output json`, all default providers. Fetch: `fetch URL
--output json --timeout 15 --content-limit 100000 --max-response-bytes 4000000`.
Cache/recovery cache off; remote telemetry off; local provider traces/artifacts on.
At most two searches and three fetches per question, with 30-second outer bounds.
Recovery is an inspected lexical refinement preserving the original intent and
site restriction. Candidate lists, diagnostics, decisions and receipts are local.

The production executable, Rust example, Cargo inputs, dataset, generated skill
and evaluation policy were verified unchanged after the run. Subsequent additions
are study reporting/assessment files, not inputs to this gate's executable or
search/fetch workflow. The final signed tree is recorded in the PR. No production
CLI contract or skill change is made. The skill installation was temporary and
existing installed skills were not overwritten.

Times below are summed subprocess wall times (including recovery/errors), not
agent end-to-end latency. Discovery and fetch are separate; no percentile or
performance inference is made for this single acceptance run.

| ID | Judgment | Discovery s | Fetch s | Total s | Evidence |
| --- | --- | ---: | ---: | ---: | --- |
| q01 | PASS | 0.773 | 0.000 | 0.773 | [answer and sources](#q01) |
| q02 | PASS | 0.848 | 1.010 | 1.858 | [answer and sources](#q02) |
| q03 | PASS | 10.986 | 0.240 | 11.226 | [answer and sources](#q03) |
| q04 | FAIL | 1.291 | 1.194 | 2.485 | [answer and sources](#q04) |
| q05 | PASS | 0.623 | 0.243 | 0.866 | [answer and sources](#q05) |
| q06 | PASS | 10.136 | 0.477 | 10.614 | [answer and sources](#q06) |
| q07 | PASS | 0.842 | 0.132 | 0.974 | [answer and sources](#q07) |
| q08 | PASS | 1.005 | 0.181 | 1.186 | [answer and sources](#q08) |
| q09 | PASS | 0.929 | 0.425 | 1.354 | [answer and sources](#q09) |
| q10 | PASS | 1.049 | 0.000 | 1.049 | [answer and sources](#q10) |

## q01

Canberra is the capital of Australia.

* [search-1](https://www.britannica.com/place/Canberra): “Canberra, federal capital of the Commonwealth of Australia. It occupies part of th…” Full passage: `q01/search-1/stdout`.

The explicit snippet fully supports this narrow answer, so direct fetches were deliberately skipped.

## q02

Air molecules scatter sunlight through Rayleigh scattering. Shorter visible wavelengths scatter more strongly than longer red wavelengths, redirecting blue light across the daytime sky.

* [fetch-2](https://www.rmg.co.uk/stories/space-astronomy/why-sky-blue): “The size of these molecules is much smaller than the wavelengths of visible light. The type of scattering…” Full passage: `q02/fetch-2/stdout`.

HyperPhysics failed; the Royal Observatory alternative supplied the scattering explanation.

## q03

TaskGroup waits for its tasks on exit. The first non-CancelledError failure cancels siblings and waits for them; ordinary failures are raised together in ExceptionGroup/BaseExceptionGroup. Handle matching grouped errors with except*, as the documented except* TerminateTaskGroup example shows. Cancellation is distinct: CancelledError is excluded from ordinary grouped failures, and should normally be propagated after cleanup. KeyboardInterrupt/SystemExit are re-raised specially.

* [fetch-1](https://docs.python.org/3/library/asyncio-task.html): “The first time any of the tasks belonging to the group fails with an exception other than asyncio.CancelledError,…” Full passage: `q03/fetch-1/stdout`.
* [fetch-1](https://docs.python.org/3/library/asyncio-task.html): “except* TerminateTaskGroup: pass asyn…” Full passage: `q03/fetch-1/stdout`.

Recovery restricted discovery to docs.python.org after the first search lacked the TaskGroup reference. The retained code preserves indentation and the except* example.

## q04

Partial evidence only: EXPLAIN (ANALYZE, BUFFERS, SETTINGS) reports how a query actually executed. The run did not retrieve enough detail to explain the actual-time/row measurements and the additional buffer counters; full answer withheld.

* [fetch-3](https://wiki.postgresql.org/wiki/Slow_Query_Questions): “EXPLAIN (ANALYZE, BUFFERS, SETTINGS) tells us how the query actually was executed, not just how it was planned.…” Full passage: `q04/fetch-3/stdout`.

Both current official reference fetches exited unsuccessfully with no extractable text. The official wiki succeeded, but lacked sufficient BUFFERS detail. One site-preserving recovery and all three fetch slots were used. This repeats the symptom documented in [#157](https://github.com/rafaelpierre/kestrel-rs/issues/157); no root-cause claim or silent reopening is made.

## q05

TraceQL {condA} > {condB} selects matching B spans that are immediate children of matching A spans; {condA} < {condB} selects B parents of A. The >> operator selects descendants at any depth, rather than only direct children. These structural operators return the right-hand matches.

* [fetch-1](https://grafana.com/docs/tempo/latest/traceql/construct-traceql-queries/): “These spanset operators look at the structure of a trace and the relationship between the spans. Structural operators…” Full passage: `q05/fetch-1/stdout`.

## q06

The requested learn.chatgpt.com configuration reference documents otel.trace_exporter values none, otlp-http and otlp-grpc, with endpoint, headers and HTTP protocol metadata. Supported TOML can disable trace export with [otel] followed by trace_exporter = "none". The advanced page confirms the [otel] table; its exporter examples configure event/log export, which is separate from trace_exporter.

* [fetch-1](https://learn.chatgpt.com/docs/config-file/config-reference): “otel.trace_exporter Type / Values none | otlp-http | otlp-grpc Details Select the OpenTelemetry trace exporter and provide any…” Full passage: `q06/fetch-1/stdout`.
* [fetch-2](https://learn.chatgpt.com/docs/config-file/config-advanced): “[otel] environment = "staging" # defaults to "dev" exporter = "none" # set to otlp-http or otlp-grpc to…” Full passage: `q06/fetch-2/stdout`.

Both sources were discovered on the requested learn.chatgpt.com domain. The trace key comes from the reference; the advanced page confirms the TOML table. Its log exporter is not presented as a trace exporter example. No domain substitution is claimed.

## q07

E0382 means a value is used after ownership was moved elsewhere. A non-Copy assignment transfers ownership. Pass a reference such as &s1 to let a function borrow while retaining ownership, or clone when an independent duplicate is required. Copy is appropriate only when the type and all members support it.

* [fetch-1](https://doc.rust-lang.org/error_codes/E0382.html): “A variable was used after its contents have been moved elsewhere. Erroneous code example: struct MyStruct { s:…” Full passage: `q07/fetch-1/stdout`.
* [fetch-1](https://doc.rust-lang.org/error_codes/E0382.html): “Sometimes we don’t need to move the value. Using a reference, we can let another function borrow the…” Full passage: `q07/fetch-1/stdout`.

## q08

NASA’s December 2025 summary says JWST had not seen signs of a thick atmosphere on TRAPPIST-1 b. The available data suggests b may be bare rock without an atmosphere; this is a qualified inference, not definitive proof that every atmosphere is absent. Findings about e do not substitute for b.

* [fetch-1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/): “Data has been successfully collected for all seven planets. As of December 2025, the science community has reported…” Full passage: `q08/fetch-1/stdout`.

## q09

Tempo 3.0 removes ingesters, compactor and scalable single binary mode. Microservices deployments need Kafka plus block-builders/live-stores and backend scheduler/workers; deploy alongside 2.x, switch traffic and decommission. Monolithic mode needs a configuration/binary migration and does not require Kafka. Upgrade blocks to vParquet4 or later first. Use tempo-cli migrate config (with --mode=monolithic for that mode); remove ingester, ingester_client, compactor, metrics_generator_client and local_blocks settings. During parallel operation disable 3.0 compaction in defaults and each tenant override until 2.x compactors are retired.

* [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “Grafana Tempo 3.0 introduces a new architecture that replaces ingesters with a Kafka-based ingest path. Distributors write trace…” Full passage: `q09/fetch-2/stdout`.
* [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “Your Tempo 2.x deployment uses vParquet4 or later as the block format. Tempo 3.0 doesn’t support vParquet3 or…” Full passage: `q09/fetch-2/stdout`.
* [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “tempo-cli migrate config --mode=monolithic old-config.yaml > new-config.yaml Or update the config manually: remove the ingester:, ingester_client:, compactor:, and…” Full passage: `q09/fetch-2/stdout`.
* [fetch-2](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/migrate-to-3/): “Only one compaction system can safely write to shared storage at a time. Disable compaction in the 3.0…” Full passage: `q09/fetch-2/stdout`.

## q10

The London Science Museum opens daily 10:00–18:00, with last entry at 17:15. It is closed 24–26 December.

* [search-1](https://www.sciencemuseum.org.uk/visit): “The museum is open daily from 10.00–18.00 (except for 24–26 December when the museum is closed). Last entry…” Full passage: `q10/search-1/stdout`.

The explicit snippet fully supports this narrow answer, so direct fetches were deliberately skipped.

## Validation

Release build, cargo fmt --check, Clippy with all targets/features and warnings denied, and cargo test --all-features passed. Benchmark Python suite: 63 tests, four environment-dependent skips. The two new replay checks also passed with the release example explicitly supplied. Gate failure is independent of those deterministic checks.
