# Portable query removal — issue #141

Portable parsing and lexical metadata rejection are removed. Provider query text
passes through unchanged; URL validity and existing standalone hostname restrictions
remain. BM25/ranking handles relevance. `--query-syntax`, `QuerySyntax` and
`SearchOptions.query_syntax` are removed. Saved commands and library callers must
remove them. The opt-in `--min-fetch-score` uses ordinary query tokens, without
Boolean parsing or affirmative-only/site-only exceptions. It still requires fetching.

Generated skill/help and migration documentation match this breaking change.
Streaming validation now records schema 2/passthrough; Bing experiments record
revision 3/passthrough. Historical readers retain the old labeled formats.
Quality/latency scripts default to passthrough; portable/native runner settings
exist only to replay historical executables. The pinned 4.0.0 study is explicit.

## Validation

`cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features` (205 passed; five opt-in live tests ignored),
`cargo build --release`, and all 44 Python benchmark tests pass. Regressions cover
shell quoting, provider query preservation, missing metadata, streamed acceptance,
URL/site boundaries, removed CLI flags, arbitrary provider syntax, blank single-provider
APIs, BM25 threshold behavior, benchmark labels, source identity and timeout output.
The obsolete parser tests were removed with the parser. No dependency/MSRV change.
The release binary generated and installed its skill into an isolated temporary
project/HOME; the generated option table and migration guidance match current help.

## Ten-question evidence gate: NOT PASSED (4/10)

Dataset: [canonical q01–q10](../benchmarks/codex-search-2026-09-11/queries.json).
One exact initial query per question; up to one intent-preserving recovery and
three direct fetches. Search: `--no-fetch -k 5 --min-results 10 --search-budget 5
--output json`. Fetch: `--content-limit 40000 --max-response-bytes 2000000
--timeout 15 --output json`. Cache disabled; diagnostics enabled. GPT-6/Codex
inspected all returned metadata and selected URLs from this run. Criteria are the
individual AGENTS.md evidence minima. All actual answers/abstentions and supporting
passages are retained locally, along with complete stdout/stderr and provider artifacts.

The following are subprocess seconds (capture included), not agent response time.

| ID | Grade | Returned per search | Fetch success/attempts | Search s | Fetch s | Total s | Evidence and conclusion |
| --- | --- | --- | --- | ---: | ---: | ---: | --- |
| q01 | pass | 5 | 1/1 | 0.622 | 0.387 | 1.009 | Canberra is the capital of Australia. [source 1](https://www.mappr.co/capital-cities/australia/) |
| q02 | pass | 5, 5 | 1/1 | 1.322 | 0.500 | 1.822 | The sky appears blue because atmospheric Rayleigh scattering scatters shorter visible wavelengths more strongly than longer ones; the retrieved explanation gives an inverse fourth-power wavelength dependence. [source 1](https://en.m.wikipedia.org/wiki/Rayleigh_scattering) |
| q03 | fail | 5, 0 | 0/0 | 5.667 | 0.000 | 5.667 | Abstained: no official Python evidence retrieved.  |
| q04 | fail | 0, 0 | 0/0 | 10.355 | 0.000 | 10.355 | Abstained: no PostgreSQL documentation retrieved.  |
| q05 | partial | 0, 5 | 3/3 | 6.079 | 0.890 | 6.969 | The retrieved Grafana prose distinguishes descendants at any depth from children, and describes parent/ancestor as inverse relationships. Structural results come from the right-hand side. A verified operator/query example could not be recovered. [source 1](https://grafana.com/docs/tempo/latest/solutions-with-traces/traces-diagnose-errors/) [source 2](https://grafana.com/blog/grafana-tempo-2-3-release-faster-trace-queries-traceql-upgrades/) [source 3](https://grafana.com/docs/grafana/latest/datasources/tempo/query-editor/traceql-editor/) |
| q06 | fail | 1, 0 | 1/1 | 10.300 | 0.257 | 10.558 | Abstained: no otel/trace_exporter configuration evidence. [source 1](https://learn.chatgpt.com/docs/codex/cli) |
| q07 | pass | 5 | 1/1 | 0.951 | 0.681 | 1.632 | E0382 means code uses a value after it has moved. Borrowing through a reference can avoid transferring ownership; cloning creates a separate owned duplicate when that is what the program needs. [source 1](https://doc.rust-lang.org/error_codes/E0382.html) |
| q08 | pass | 5 | 1/1 | 0.406 | 0.279 | 0.684 | NASA’s retrospective explicitly describes the state as of December 2025: Webb had not seen a thick atmosphere on TRAPPIST-1 b, and the data suggested a bare rock without an atmosphere. This is tentative; stellar contamination and the need for further observations limit certainty. [source 1](https://science.nasa.gov/mission/webb/science-overview/science-explainers/what-is-webb-revealing-about-the-trappist-1-system/) |
| q09 | fail | 5, 5 | 0/0 | 1.098 | 0.000 | 1.098 | Abstained: no Tempo 3.0 breaking-change documentation.  |
| q10 | fail | 5, 5 | 0/0 | 1.184 | 0.000 | 1.184 | Abstained: no London Science Museum opening hours.  |

17 searches and 8 successful direct fetches. Nearest-rank search
median 0.729s, p95 5.196s;
total measured subprocess work 40.977s. These are one live
acceptance window, not a causal speedup or improvement over historical portable runs.
Provider response variance, blocks, missing relevant candidates and lost extracted
code remain. q05 demonstrates that successful fetches alone are insufficient.
q08 uses NASA's January 2026 retrospective explicitly covering December 2025.

## Provenance and retained artifacts

- Base revision: `f3540ad1c5c24bff630a6f4f9784bf779ae6891e` plus tracked-diff SHA-256 `52da3475eea2299003d03736300b3f03703ef1492a760c358b72eea13de730b0`.
- Release binary: `target/release/kestrel`, `kestrel 4.1.0`, SHA-256 `09464a90b18968571617ac4ebc1ac60e0d652f690414164417f1cb0def7c83d2`.
- Executable inputs (ordered src files plus Cargo.toml/Cargo.lock): SHA-256 `57b58332c74e525bfae630c79e438fbb93d2b7af5924c6fdb9167949e19099c8`; exact paths/hash recipe recorded with metadata.
- Dataset SHA-256: `fcc8aa7ff7d9f4249c6be87b5b4a8533d2f5b839ea7bb01abb7b87891a58c9d6`.
- Full local evidence: `/tmp/kestrel-141-removal-final` (`metadata.json`, `policy.json`, `calls.jsonl`, `answers.jsonl`, `summary.json`, selection/recovery plans, provider artifacts).
- Preliminary capture `/tmp/kestrel-141-removal-gate` remains separate; final rerun follows API/provenance fixes. It is not pooled.
- Later edits are benchmark-test/report documentation only; executable inputs, generated skill, dataset and evaluation policy are unchanged. The signed commit's complete tree is checked against the tested worktree before publication.

The PR remains draft. Existing retrieval/extraction work (#32, #75, #79) remains;
removing portable filtering does not establish 10/10 evidence coverage or repair
provider responses. No installed user executable or skill was overwritten.
