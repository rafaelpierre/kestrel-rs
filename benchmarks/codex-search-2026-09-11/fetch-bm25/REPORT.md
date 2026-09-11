# Search benchmark: fetching and BM25 enabled

30 Kestrel searches: the same 10 queries, three rounds, executed sequentially. Built-in search was not rerun, as requested. Its saved five-round baseline is reused.

Measured configuration: Kestrel 1.1.1, fanout, provider quorum 1, fetching and BM25 enabled, top-k 5, up to 15 candidates, 2,000 extracted characters per page, fetch budget 20 seconds, fetch concurrency **5**. Other settings were CLI defaults.

The runner now explicitly uses fetch concurrency **10** for future runs, as requested after this batch completed. The saved measurements were not rerun at concurrency 10.

| Metric | Built-in, saved | Kestrel no-fetch, saved | Kestrel fetch + BM25 |
|---|---:|---:|---:|
| Runs | 50.000 | 50.000 | 30.000 |
| Median seconds | 1.613 | 0.546 | 1.472 |
| p95 seconds (nearest rank) | 2.783 | 0.662 | 2.439 |
| Direct relevance precision@5 | 81.2% | 4.8% | 10.0% |
| Runs with a directly relevant result | 50.000 | 3.000 | 3.000 |
| Empty runs | 0.000 | 1.000 | 0.000 |

| Query | Built-in median s (saved) | Kestrel no-fetch median s (saved) | Fetch + BM25 median s | Direct hits / 5, mean |
|---|---:|---:|---:|---:|
| What is the capital of Australia? | 1.604 | 0.642 | 1.316 | 0.0 |
| why is the sky blue Rayleigh scattering | 1.788 | 0.553 | 2.006 | 0.0 |
| Python asyncio TaskGroup exception handling ExceptionGroup | 1.623 | 0.525 | 1.342 | 0.0 |
| site:postgresql.org EXPLAIN ANALYZE BUFFERS | 1.697 | 0.563 | 1.478 | 0.0 |
| site:grafana.com Tempo TraceQL parent child spans | 1.728 | 0.507 | 1.610 | 0.0 |
| site:learn.chatgpt.com Codex otel trace_exporter | 1.576 | 0.596 | 1.810 | 0.0 |
| Rust E0382 use of moved value how to fix | 1.597 | 0.637 | 1.069 | 0.0 |
| James Webb TRAPPIST-1 b atmosphere observations 2025 | 1.662 | 0.517 | 1.414 | 0.0 |
| Grafana Tempo 3.0 release notes breaking changes | 1.727 | 0.533 | 1.465 | 0.0 |
| London Science Museum opening hours | 1.560 | 0.532 | 1.191 | 5.0 |

Quality assessment

The same 0–2 relevance rubric and earlier judgments for repeated query/URL pairs were used. New URLs were reviewed using titles, URLs and snippets, keeping fetched body text out of the relevance score. Score 2 counts as directly relevant; missing top-five slots count as zero. This is a single-assistant, non-blind assessment of relevance, not a fact-check of page contents.

Fetching and BM25 changed ordering and removed candidates but did not resolve the broad-keyword retrieval failures in this environment. Only the museum-hours query yielded directly relevant results. The James Webb query returned a James-band discography; PostgreSQL returned definitions of ‘explain’; TraceQL returned music-tempo pages. These results suggest a query-handling/provider problem, whose cause has not been diagnosed. This is not evidence that BM25 generally worsens search.

The built-in baseline also has usability limitations: translated Codex pages, duplicate documentation and older PDFs. See ../REPORT.md for the original analysis.

Timing limits

Complete-call elapsed time includes the shell wrapper and any automatic polling; a monotonic subprocess timer is saved separately. No model thinking or manual polling gaps were included. Built-in response size, internal fetching/ranking, and upstream caches are not controllable. The baseline was collected earlier, and the sample sizes differ (50 vs 30). This is a descriptive local comparison, not a controlled causal estimate of enabling BM25 or fetching.

Files: results.json (raw output and measured settings), judgments.json (per-URL scores), summary.json (metrics), analyze.py (recompute report), run-kestrel.py (future runs at concurrency 10).
