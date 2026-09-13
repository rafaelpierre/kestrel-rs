# Issue #35 — ten-question evidence gate

**Gate NOT PASSED: 3/10 individual passes.** Every q01–q10 was run; partial evidence and missing official pages fail. No source from browser comparison or another search tool was used to fill gate gaps.

[Canonical dataset](../benchmarks/codex-search-2026-09-11/queries.json). Related existing investigations: [#35](https://github.com/rafaelpierre/kestrel-rs/issues/35), [#32](https://github.com/rafaelpierre/kestrel-rs/issues/32) (Bing fidelity), [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148) (evidence gaps).

## Declared workflow

Assessor: Codex GPT-6 session, AGENTS.md individual evidence rubric. Exact manifest query first; all nine default providers, `search QUERY --no-fetch --no-rank --min-results 10 -k 10 --search-budget 5 --output json`. At most two discovery calls and three direct fetches per question. Recovery adds lexical specificity or official-domain restriction while preserving entity/version/date and all existing restrictions. Recovery queries and reasons are retained in `recovery-reasons.json` and `q10-recovery-reason.json`.

Selective `fetch URL --timeout 10 --content-limit 60000 --max-response-bytes 2000000 --output json`. Outer process deadline 30 seconds. Cache off; fresh CLI processes with randomized production headers. Captures enabled. All returned candidates inspected; only selected URLs from this run fetched. No search/fetch policy adjustment mid-run. Median convention, if used, is middle/mean middle two; no percentile performance comparison is made.

Times below are summed external subprocess wall times, including recovery and startup/capture overhead. They are neither agent end-to-end latency nor pure network latency. Discovery and fetch remain separate.

| ID | Judgment | Discovery s | Fetch s | Total s | Evidence and rationale |
| --- | --- | ---: | ---: | ---: | --- |
| q01 | PASS | 5.800 | 0.000 | 5.800 | Canberra is the capital of Australia. |
| q02 | PASS | 3.694 | 1.076 | 4.770 | Air molecules Rayleigh-scatter sunlight; shorter visible wavelengths scatter more strongly than longer red wavelengths (approximately inverse fourth power of wavelength), making the diffuse daytime sky blue. |
| q03 | FAIL | 12.979 | 0.000 | 12.979 | No supported answer: initial results were third-party explanations and the official-domain recovery returned no candidates. Required official TaskGroup/cancellation/grouped-exception/except* evidence is missing. |
| q04 | FAIL | 12.408 | 3.267 | 15.675 | Partial evidence only: official snippets describe BUFFERS as reporting I/O and buffer hits, reads, dirties and writes. Both selected official pages returned no extractable text; retrieved snippets do not establish the required ANALYZE execution/reporting explanation. |
| q05 | FAIL | 15.901 | 0.000 | 15.901 | No supported answer: both searches returned no candidates; immediate parent/child operator direction and descendant distinction remain unsupported. |
| q06 | FAIL | 17.408 | 0.000 | 17.408 | No supported answer: both learn.chatgpt.com-restricted searches returned no candidates. No configuration syntax or documented redirect was retrieved; domain restriction was preserved. |
| q07 | FAIL | 13.867 | 0.000 | 13.867 | No supported answer: initial farming/forum results were unrelated, and official Rust domain recovery returned no candidates. No official E0382 explanation or repair retrieved. |
| q08 | PASS | 5.687 | 4.915 | 10.602 | A January 7, 2025 report describes uncertainty for TRAPPIST-1 b: earlier JWST data favored an airless rock, while additional data allowed either an airless recently resurfaced surface or a less likely thick hazy CO2 atmosphere. The article reports a December 16 study, not a newly dated 2025 observation. NASA’s January 2026 retrospective explicitly summarizes knowledge as of December 2025: no signs of a thick atmosphere on b; a bare rock remains possible, with stellar contamination and further observations limiting certainty. No definitive atmosphere detection is claimed. |
| q09 | FAIL | 16.870 | 0.000 | 16.870 | No supported answer: initial results were Yahoo Mail pages; official Grafana recovery returned no candidates. No Tempo 3.0 release/migration changes retrieved. |
| q10 | FAIL | 14.110 | 0.000 | 14.110 | No supported answer: initial results concerned London generally; official museum domain recovery returned no candidates. Opening hours and holiday qualifications cannot be stated. |

## Retained evidence and synthesized answers

### q01 — Australia capital

Canberra is the capital of Australia.

Local evidence: `q01-search1/stdout.json` (relative to `gate-v1`). Skipped: snippet sufficient.

Selected source: [https://www.britannica.com/place/Canberra](https://www.britannica.com/place/Canberra). Supporting passage: “Canberra, federal capital of the Commonwealth of Australia.”.

### q02 — Rayleigh scattering blue sky

Air molecules Rayleigh-scatter sunlight; shorter visible wavelengths scatter more strongly than longer red wavelengths (approximately inverse fourth power of wavelength), making the diffuse daytime sky blue.

Local evidence: `q02-fetch3/stdout.json` (relative to `gate-v1`). See retained command exit codes and stdout for extraction separately from usefulness.

Selected source: [https://en.m.wikipedia.org/wiki/Rayleigh_scattering](https://en.m.wikipedia.org/wiki/Rayleigh_scattering). Supporting passage: “shorter (blue) wavelengths are scattered more strongly than longer (red) wavelengths”.

### q03 — Python asyncio TaskGroup exception handling ExceptionGroup

No supported answer: initial results were third-party explanations and the official-domain recovery returned no candidates. Required official TaskGroup/cancellation/grouped-exception/except* evidence is missing.

Local evidence: `q03-search2/stdout.json` (relative to `gate-v1`). Skipped: no qualifying candidate.

### q04 — site:postgresql.org EXPLAIN ANALYZE BUFFERS

Partial evidence only: official snippets describe BUFFERS as reporting I/O and buffer hits, reads, dirties and writes. Both selected official pages returned no extractable text; retrieved snippets do not establish the required ANALYZE execution/reporting explanation.

Local evidence: `q04-search2/stdout.json; q04-fetch1/stderr.txt; q04-fetch2/stderr.txt` (relative to `gate-v1`). See retained command exit codes and stdout for extraction separately from usefulness.

Selected source: [https://www.postgresql.org/docs/current/using-explain.html](https://www.postgresql.org/docs/current/using-explain.html). Supporting passage: “additional detail about I/O operations performed during the planning and execution”.

### q05 — site:grafana.com Tempo TraceQL parent child spans

No supported answer: both searches returned no candidates; immediate parent/child operator direction and descendant distinction remain unsupported.

Local evidence: `q05-search1/stdout.json; q05-search2/stdout.json` (relative to `gate-v1`). Skipped: no qualifying candidate.

### q06 — site:learn.chatgpt.com Codex otel trace_exporter

No supported answer: both learn.chatgpt.com-restricted searches returned no candidates. No configuration syntax or documented redirect was retrieved; domain restriction was preserved.

Local evidence: `q06-search1/stdout.json; q06-search2/stdout.json` (relative to `gate-v1`). Skipped: no qualifying candidate.

### q07 — Rust E0382 use of moved value

No supported answer: initial farming/forum results were unrelated, and official Rust domain recovery returned no candidates. No official E0382 explanation or repair retrieved.

Local evidence: `q07-search1/stdout.json; q07-search2/stdout.json` (relative to `gate-v1`). Skipped: no qualifying candidate.

### q08 — James Webb TRAPPIST-1 b atmosphere observations 2025

A January 7, 2025 report describes uncertainty for TRAPPIST-1 b: earlier JWST data favored an airless rock, while additional data allowed either an airless recently resurfaced surface or a less likely thick hazy CO2 atmosphere. The article reports a December 16 study, not a newly dated 2025 observation. NASA’s January 2026 retrospective explicitly summarizes knowledge as of December 2025: no signs of a thick atmosphere on b; a bare rock remains possible, with stellar contamination and further observations limiting certainty. No definitive atmosphere detection is claimed.

Local evidence: `q08-fetch8/stdout.json; q08-fetch3/stdout.json` (relative to `gate-v1`). See retained command exit codes and stdout for extraction separately from usefulness.

Selected source: [https://skyandtelescope.org/astronomy-news/trappist-1b-atmosphere-debated-some-stars-take-their-time-forming-planets/](https://skyandtelescope.org/astronomy-news/trappist-1b-atmosphere-debated-some-stars-take-their-time-forming-planets/). Supporting passage: “either the planet’s surface is indeed airless but recently resurfaced”.

### q09 — Grafana Tempo 3.0 release notes breaking changes

No supported answer: initial results were Yahoo Mail pages; official Grafana recovery returned no candidates. No Tempo 3.0 release/migration changes retrieved.

Local evidence: `q09-search1/stdout.json; q09-search2/stdout.json` (relative to `gate-v1`). Skipped: no qualifying candidate.

### q10 — London Science Museum opening hours

No supported answer: initial results concerned London generally; official museum domain recovery returned no candidates. Opening hours and holiday qualifications cannot be stated.

Local evidence: `q10-search1/stdout.json; q10-search2/stdout.json` (relative to `gate-v1`). Skipped: no qualifying candidate.

## Provenance and limitations

Tested base revision `e7527719feff0265ea34ee7334f289f076162b76`, base tree `900eba9b6c8058a0b075ea12c665b3ae0cd434f5`; version `kestrel 6.0.0`. Binary `/Users/rafaelpierre/projects/kestrel-rs-issue-35/target/release/kestrel`; SHA-256 `514df0ef84999eb6cf968e57d49a43a0b9797302cd86cc68c894033874258e36`. Dataset SHA-256 `cb5887c8cf040603cdc47ff28c070eed6f16eb2251c58333a9656d7abfd0d5c9`.

Only cfg(test) Rust code, offline benchmark scripts and reports differ from the base; shipped executable, skill and dataset inputs are unchanged. The same release binary was used for discovery, fetching and temporary project skill installation. The PR commit adds documentation of this run, not executable input changes. The saved tracked patch identifies test changes at run start; benchmark/report files are captured in the final PR tree.

Full argv, timestamps, exits, stdout/stderr, raw provider traces, full retained page text, policy, patch and judgments are local under `/Users/rafaelpierre/projects/kestrel-rs-issue-35/benchmarks/results/issue-35/gate-v1/`. Successful extraction is recorded separately from usefulness: q02 and q08 extracted useful text; both q04 fetch attempts failed; q01 needed no fetch. Other rows skipped fetching because no qualifying official candidate was found.

This is one live acceptance observation. Provider blocks, empty retrieval, unrelated candidates and failed extraction remain failures. No readiness, merge eligibility or statistically reliable performance claim follows from 3/10.
