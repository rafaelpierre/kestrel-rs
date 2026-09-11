# Search benchmark: fetching and BM25 disabled

Tested 2026-09-11: 10 queries × 5 rounds × 2 tools = 100 sequential calls. Kestrel 1.1.1 used fanout, provider quorum 1, no fetching, no ranking, top 5. Built-in used identical query strings (including site: filters), response_length=short. Tool order alternated by query and round.

| Metric | Built-in | Kestrel |
|---|---:|---:|
| Median seconds | 1.613 | 0.546 |
| p95 seconds (nearest rank) | 2.783 | 0.662 |
| Mean seconds | 1.903 | 0.598 |
| Directly relevant results / 250 | 203.000 | 12.000 |
| Runs with a directly relevant result / 50 | 50.000 | 3.000 |
| Empty responses / 50 | 0.000 | 1.000 |
| Domain-compliant results / 75 restricted-query slots | 75.000 | 0.000 |

Direct relevance precision@5: built-in 81.2%; Kestrel 4.8%. Graded relevance (0–2): built-in 90.6%; Kestrel 6.4%. Missing results count as zero. Scores reflect one assistant review of returned titles, URLs and excerpts, not independent verification of full-page accuracy.

| Query | Built-in median s | Kestrel median s | Built-in direct hits / 5 | Kestrel direct hits / 5 |
|---|---:|---:|---:|---:|
| What is the capital of Australia? | 1.604 | 0.642 | 1.0 | 0.0 |
| why is the sky blue Rayleigh scattering | 1.788 | 0.553 | 5.0 | 0.0 |
| Python asyncio TaskGroup exception handling ExceptionGroup | 1.623 | 0.525 | 5.0 | 0.0 |
| site:postgresql.org EXPLAIN ANALYZE BUFFERS | 1.697 | 0.563 | 5.0 | 0.0 |
| site:grafana.com Tempo TraceQL parent child spans | 1.728 | 0.507 | 2.0 | 0.0 |
| site:learn.chatgpt.com Codex otel trace_exporter | 1.576 | 0.596 | 5.0 | 0.0 |
| Rust E0382 use of moved value how to fix | 1.597 | 0.637 | 5.0 | 0.0 |
| James Webb TRAPPIST-1 b atmosphere observations 2025 | 1.662 | 0.517 | 2.6 | 0.0 |
| Grafana Tempo 3.0 release notes breaking changes | 1.727 | 0.533 | 5.0 | 0.0 |
| London Science Museum opening hours | 1.560 | 0.532 | 5.0 | 2.4 |

First-round median: built-in 1.635s, Kestrel 0.513s. Repeat-round median: built-in 1.613s, Kestrel 0.562s. First does not mean cold cache; upstream caches were not controlled.

## Quality observations

- Kestrel retained Bing results in every nonempty response. Most results matched broad keywords or a different entity: Capital FM for Australia’s capital, music tempo for TraceQL, the Rust game for a compiler error, and the band James for James Webb. The full query was passed as one subprocess argument; the root cause of the poor provider results was not diagnosed.
- Kestrel found directly relevant museum pages in rounds 3–5; round 1 returned generic London pages, round 2 was empty.
- Built-in returned relevant official documentation and research, but some results were duplicates, translated Codex docs, old museum PDFs, or papers outside the requested year. The rubric scores topical relevance; these usability limitations remain.

## Limits and reproduction

These are measurements of the tools as exposed in this environment, not isolated search-engine latency. Elapsed time is recorded around each complete tool invocation, including shell/protocol overhead. Kestrel subprocess time is also saved separately. Built-in returns more than five results and longer excerpts; only its first five displayed results are scored, but timing includes the full response. No explicit language, region, cache reset or built-in domain parameter was applied. Identical site: query text was used for both tools.

Quality is a non-blind, single-reviewer judgment. Repeated identical results reuse the same URL/query judgment. Precision@5 counts score=2 only; score=1 is partial/background or a date mismatch. A likely relevant page is not proof its content is correct. The 50 observations per tool contain repeated queries and should not be treated as 50 independent information needs.

- `results.json`: raw responses, normalized top-five results, timings and exact queries.
- `judgments.json`: explicit per-query/URL scores and rationales.
- `summary.json`: computed metrics.
- `analyze.py`: recompute with `python3 benchmarks/search/analyze.py`.
- `run-kestrel.py`: invoke the original Kestrel configuration for one query.
