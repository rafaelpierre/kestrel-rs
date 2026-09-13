# Frozen-pool candidate-cap study, version 2

Issue #75’s remaining experiment now has **63/63 comparable live-fetch replays**: seven frozen pools, three cap treatments and three counterbalanced repeats. Seven other queries failed the 15-candidate pool requirement and remain explicit negative observations. No production defaults or CLI/search/fetch contracts changed. The separate mandatory evidence gate is **NOT PASSED (3/10)**; this work is not ready to merge.

The [version 1 pilot](../quality-study-20260913/README.md) remains unchanged. This experiment measures fetching/ranking conditional on frozen metadata, not complete search latency or a guarantee of useful retrieval. Related work: [#76](https://github.com/rafaelpierre/kestrel-rs/issues/76), [#84](https://github.com/rafaelpierre/kestrel-rs/issues/84), [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148).

## Results

Each cap has 21 calls (three repeats on seven queries). All returned five slots; valid-empty and process-error rates were both zero. Every expected metadata prefix matched. This is a successful comparability check, not an evidence-quality pass. Nearest-rank p95 is an exploratory order statistic, not a reliable population tail estimate.

| Cap | Process p50 / p95, s | Command p50 / p95, s | Fetch p50 / p95, s | Metadata P@5 | Page evidence slots |
| --- | ---: | ---: | ---: | ---: | ---: |
| 5 | 1.189 / 5.421 | 1.178 / 4.005 | 1.046 / 3.868 | 0.229 | 14.3% |
| 10 | 1.489 / 4.901 | 1.477 / 4.887 | 1.258 / 4.752 | 0.314 | 20.0% |
| 15 | 2.040 / 5.153 | 2.029 / 5.138 | 1.891 / 5.003 | 0.371 | 24.8% |

Pooled evidence gains came only from Canberra and Rayleigh-scattering pages. For PostgreSQL, increasing the cap pushed the two direct reference pages out of the final five in favor of mailing-list results; none of the returned bodies supported the complete requested explanation. The duet query returned plumbing listings. Tempo 3.0 returned agriculture/economics pages. The Science Museum results contained visitor descriptions or another museum’s hours, not the requested hours. Successful extraction did not repair these gaps.

The cap-15 minus cap-5 paired mean process difference was +0.729 s; the exploratory query-cluster bootstrap interval was −0.125 to +1.694 s. The P@5 difference was +0.143 (−0.114 to +0.371); evidence-slot difference was +0.105 (0.000 to +0.276). These intervals resample seven query means, 1,000 times, seed 75; endpoints are ordered draws 25 and 975. They do not account for assessor disagreement, selection bias, multiple comparisons or correlated upstream behavior. Per-query observed timing ranges and all comparisons are in [results.json](results.json).

**No latency-policy recommendation follows.** At study start the shared host reported load averages 243.04 / 168.98 / 88.56; unrelated builds were active. Local benchmark process concurrency was one, but host contention was uncontrolled. The unevenly relevant pools, short body cap, one unblinded assessor and single network observation limit generalization.

## Discovery and pool failures

All 14 exact queries from [quality-queries-v1.json](../quality-queries-v1.json) received two sequential discoveries. No recovery, query substitution, artificial padding or subsequent pool refill was used. First occurrences win after tracking/fragment/trailing-slash URL normalization; the ordered union is frozen before any treatment. A two-search union is not a promised single-search output.

| Query | Unique pool | Eligible | Discovery process total, s |
| --- | ---: | --- | ---: |
| music-bss | 10 | no | 24.243 |
| music-duet | 20 | yes | 22.534 |
| music-metric | 10 | no | 23.055 |
| music-sonic | 10 | no | 22.062 |
| q01 | 26 | yes | 23.455 |
| q02 | 30 | yes | 14.232 |
| q03 | 10 | no | 22.565 |
| q04 | 20 | yes | 23.005 |
| q05 | 0 | no | 22.516 |
| q06 | 7 | no | 21.618 |
| q07 | 48 | yes | 13.620 |
| q08 | 10 | no | 21.175 |
| q09 | 17 | yes | 20.680 |
| q10 | 38 | yes | 12.006 |

All seven undersized queries are excluded from cap means, not counted as zero-quality or zero-latency replay observations. All 28 discovery calls, including empty results, remain in the raw artifacts. Metadata titles, URLs and snippets were inspected for all pools. The cap treatment only uses the first 15 unique candidates of eligible pools.

## Protocol and provenance

- Discovery: all nine engines in CLI order, passthrough query text, minimum/top-k 30, no fetch/rank, explicit 10-second search budget, 30-second outer timeout, region empty, time filter any, provider tracing/artifacts enabled.
- Replay: caps 5/10/15, top-k five, hybrid ranking, pre-ranking and score gate disabled; 5-second fetch budget, 10-second request timeout, fetch/parse concurrency ten, 2,000 characters and 1,000,000 decoded bytes per page. PDF URLs skipped after prefix selection. Each process initializes fresh clients; no page cache. Upstream cache and body changes remain uncontrolled.
- Three rounds rotate each treatment through all positions per query; one subprocess at a time, 0.25 seconds minimum pacing. Command time includes pool loading/client setup and excludes discovery. Process time additionally includes runtime startup, output and shutdown. Fetch time is separately retained. Timings include failures/censoring when present; this run had no process failures.
- Assessor: Codex GPT-6, single unblinded assessment. Relevance: 0 off-topic, 1 background, 2 directly addresses the intent. P@5 counts only 2. Evidence: retained body supports the requested fact/explanation, not merely extraction success. Judgments are bound to query, URL and exact returned-content SHA-256; missing bodies earn no page evidence. All 315 returned slots are covered by 69 distinct [judgments](judgments.json). This is not complete agent-answer evaluation.
- Source: `e7527719feff0265ea34ee7334f289f076162b76`, production tree `900eba9b6c8058a0b075ea12c665b3ae0cd434f5`, Kestrel 6.0.0. Measured implementation hashes are in the results manifest; new benchmark files were uncommitted at measurement. Production sources/Cargo inputs match the later evidence-gate build.
- Rust/Cargo 1.96.0, aarch64-apple-darwin, LLVM 22.1.6. Rust edition 2024 and declared MSRV 1.89 remain unchanged; the new example uses existing APIs and no new dependencies.
- Study build: `cargo build --release --locked --example fetch_cap_replay --bin kestrel`. Frozen search binary SHA-256 `48249fd15fab51923342e49fd12068da3f231c6c7d0c60de2d5e43cc4ee537f7`; replay SHA-256 `47fb7d6d86021e11effb95a3d91742373a030c395a0b3642078e49ed2d0693b8`.
- Gate build: `cargo build --release --locked`; binary SHA-256 `514df0ef84999eb6cf968e57d49a43a0b9797302cd86cc68c894033874258e36`. These are distinct build artifacts; do not conflate their hashes. Both source identities were checked. The temporary generated skill belongs to the gate binary.

Raw study artifacts are retained locally at `/Users/rafaelpierre/projects/kestrel-rs-75-fixed-pool/benchmarks/results/issue-75-fixed-pool-v2`. They contain frozen executables, full argv/stdout/stderr, timestamps, provider traces, discoveries, pools and all retained page text. Public results omit raw body text and normalize local argv prefixes; public hashes bind them to local captures. Historical version 1 artifacts were not altered.

## Evidence gate

The gate is independent of the cap comparison. It starts each [canonical q01–q10 query](../codex-search-2026-09-11/queries.json) exactly, as required by AGENTS.md; this takes precedence over the generated skill’s general FTS-reformulation advice. Recovery uses keyword refinement or an unchanged retry without relaxing intent. The final generated skill was installed in a temporary project and inspected.

Policy: metadata discovery with `--no-fetch --no-rank -k 20 --min-results 20 --search-budget 10 --output json`; selected direct fetches with `--timeout 15 --content-limit 100000 --max-response-bytes 4000000 --output json`; 30-second process bound, cache off, at most two searches/three fetches each. Assessor and grading criteria were declared before execution. All candidates were inspected; no remembered URLs or other search tools supplied evidence.

| ID | Judgment | Search / fetch / total tool seconds | Evidence or blocker |
| --- | --- | ---: | --- |
| q01 | PASS | 10.163 / 0.000 / 10.163 | [search-1](https://www.mappr.co/capital-cities/australia/) |
| q02 | PASS | 10.177 / 0.466 / 10.643 | [fetch-1](https://en.m.wikipedia.org/wiki/Rayleigh_scattering) |
| q03 | FAIL | 12.278 / 0.000 / 12.278 | Initial results included third-party guides but no official TaskGroup API reference; official-docs recovery returned zero results. |
| q04 | FAIL | 11.830 / 0.000 / 11.830 | Both exact site-restricted searches returned zero candidates; no PostgreSQL source could be selected. |
| q05 | FAIL | 12.360 / 0.000 / 12.360 | Both original and structural-operator recovery searches returned zero candidates. |
| q06 | FAIL | 12.340 / 0.000 / 12.340 | Both exact learn.chatgpt.com searches returned zero candidates; no domain substitution or undocumented redirect was used. |
| q07 | FAIL | 12.126 / 0.000 / 12.126 | Initial results were unrelated adult-site metadata; official Rust FTS recovery returned zero candidates. |
| q08 | PASS | 10.144 / 0.266 / 10.410 | [fetch-1](https://skyandtelescope.org/astronomy-news/trappist-1b-atmosphere-debated-some-stars-take-their-time-forming-planets/) |
| q09 | FAIL | 6.718 / 0.000 / 6.718 | Initial results were general Grafana pages or Grafana UI releases, not Tempo 3.0; official-site recovery returned zero candidates. |
| q10 | FAIL | 4.270 / 0.000 / 4.270 | Initial results were unrelated calculator pages; museum-specific official-domain recovery returned zero candidates. |

Full synthesized answers, supporting excerpts, inspection/skip rationales and provenance are in [gate.json](gate.json). These are subprocess wall times, not agent end-to-end latency. q01’s snippet fully establishes Canberra; q02’s fetched text establishes Rayleigh scattering and shorter-wavelength preference; q08’s January 7, 2025 report describes an airless/resurfaced surface versus a less-likely hazy CO2 atmosphere, without claiming a detection. All seven failures remain failures; baseline/provider failures are not waivers.

Full gate artifacts remain at `/Users/rafaelpierre/projects/kestrel-rs-75-fixed-pool/benchmarks/results/issue75-gate-v4`. Initialization v1 failed because a reused launcher built in the wrong worktree; v2 was an initialization-only attempt superseded by v3 after benchmark edits. Both remain retained. The complete [v3 gate](gate-v3.json) scored 3/10. After correcting the reporting label for scheduled versus completed page counts, v4 repeated all ten questions (17 searches and two fetches) and independently scored 3/10. No earlier row was substituted into v4. The local launcher is snapshotted with its hash in gate.json. The final report generator SHA-256 is `4354b2d2168d35a62fe737eb33ca0336e06c38aa425f2711f0c73acf48718e29`; its post-measurement count-label correction changes neither executable nor experimental inputs. No earlier successful rows were cherry-picked. Only documentation and evidence exports were added after this run; executable inputs, generated skill, canonical dataset and assessment policy are unchanged.

## Reproduction and validation

Run the commands in [the benchmark methodology](../README.md#frozen-metadata-live-candidate-cap-comparison-issue-75-version-2) using a fresh directory. `report_fixed_pool.py` recomputes raw pool, process, command, fetch and ranking metrics and leaves semantic judgments unknown until assessed. The public data stores the completed judgments separately. Verify the published quality arithmetic:

```python
import json
from pathlib import Path
root = Path('benchmarks/quality-study-20260913-v2')
data = json.loads((root / 'results.json').read_text())
js = json.loads((root / 'judgments.json').read_text())
judgments = {j['id']+'|'+j['url']+'|'+j['content_sha256']: j for j in js}
for row in data['measurements']:
    selected = [judgments[r['judgment']] for r in row['results']]
    assert row['p_at_5'] == sum(j['relevance'] == 2 for j in selected) / 5
    assert row['evidence_at_5'] == sum(j['evidence'] for j in selected) / 5
```

Local checks passed: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features` (224 passed, five opt-in/live ignored), release builds, and 49 Python benchmark tests including five new focused tests with the real replay executable. The local HTTP fixture verifies exact 5/10/15 page-request sets, five returned results, duplicate/undersized/stale pool rejection before requests, and PDF skipping. No production CLI or generated-skill contract was changed. The only readiness blocker claimed here is the unsatisfied evidence gate; remote checks are reported separately in the PR.
