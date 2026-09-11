# Search quality and latency investigation — 2026-09-11

The implementation adds bounded retrieval, observable provider failures, stricter domain filtering, optional ranking policies, six opt-in providers, randomized browser headers, and explicit HTTP connection reuse. Defaults for engines and ranking remain unchanged: the evidence does not yet establish quality parity with native search.

## Baseline and diagnosis

All 12 files in the supplied COPY-MANIFEST were verified unchanged. Historical native search measured median 1.613 s, p95 2.783 s and precision@5 81.2%; Kestrel without fetching measured 0.546 s, 0.662 s and 4.8%, and fetched BM25 measured 1.472 s, 2.439 s and 10%. Those runs used fetch concurrency 5; the subsequently edited runner is not evidence for concurrency 10.

A fresh direct Bing request reproduces unrelated dictionary results for the complete PostgreSQL EXPLAIN query. The full query appears in the returned page title/input, and the unrelated URLs are already in raw HTML: this failure occurs upstream of Kestrel's parser. Its underlying engine/environment cause remains unresolved. DuckDuckGo challenges/timeouts and Yahoo HTTP errors explain why quorum 1 predominantly retained Bing. Increasing quorum without a deadline produced roughly 46-second tails in exploratory runs.

## Implemented changes

- Shared absolute search deadlines include queueing and retries, preserving completed results. Fallback continues after valid empty responses. Diagnostics distinguish failures, filtered results, cancellations and retry counts; optional raw traces include negotiated HTTP versions.
- Unambiguous positive hostname `site:` constraints are enforced locally, including hostname boundaries. Complex boolean/quoted/path forms are preserved without claiming equivalent local enforcement. Original engine ranks survive filtering.
- Provider, snippet, body, hybrid and reciprocal-rank fusion policies are available explicitly. Experimental lexical ranking uses positive IDF, preserves version tokens, ignores synthetic source prefixes and retains candidates when body fetching fails.
- Dogpile, Ecosia, Swisscows, Yep, Qwant and Mojeek have separate transport/parsing adapters, query-encoding tests and documented filter limitations. AOL is excluded. See [provider contracts](../../docs/search-providers.md) and [issue 6](https://github.com/rafaelpierre/kestrel-rs/issues/6).
- Coherent browser/OS header profiles are randomly selected for each client and retained across its requests/retries. Existing primp supplies profiles; no additional fake-useragent dependency is needed.
- HTTPS negotiates HTTP/2 using ALPN where supported, with HTTP/1.1 fallback. Pools retain up to 10 idle connections per host for 90 seconds; TCP keepalive and active HTTP/2 keepalive use 30-second intervals. Cloned library clients share pools. Separate CLI processes cannot share connections.
- Reproducible benchmark tooling freezes the tested binary, records source/input hashes, alternates conditions, records failures and separates cache conditions. The scorer requires explicit judgments and reports missing slots and query-cluster confidence intervals.

## Completed live evidence

These are sequential development-set measurements in this environment, not causal estimates of the effect of random headers or HTTP/2. Native search was not rerun concurrently.

| Condition | Calls | Median | p95 | Returned slots |
| --- | ---: | ---: | ---: | ---: |
| Swisscows, no fetch, budget 3 s | 30 | 0.449 s | 1.354 s | 150/150 |
| Bing, no fetch, budget 3 s | 30 | 0.583 s | 1.241 s | 108/150 |
| Original engines, quorum 1, budget 3 s | 30 | 0.748 s | 3.166 s | 107/150 |
| Original engines, quorum 2, budget 3 s | 30 | 3.157 s | 3.216 s | 108/150 |
| Original engines, all, budget 3 s | 30 | 3.163 s | 3.213 s | 108/150 |

All 30 Swisscows response traces report HTTP/2.0. DuckDuckGo and Yahoo each failed all 30 calls in the pinned original-provider batch. CLI wall time includes startup/output overhead beyond the search deadline. More quorum did not recover meaningful coverage here.

Manual review of the Swisscows top five gives 66% precision@5, a direct hit in 27/30 calls, no duplicate slots and complete simple-domain compliance. The query-bootstrap 95% interval is wide: 46–84%. Offline replay on the same captured candidates gives 76% for snippet/hybrid after preserving dotted version tokens (interval 54–92%). This is a tuned development-set result, not held-out validation; the wrong-planet query remains unsolved. Body ranking over no-fetch candidates is not evidence about fetched-body BM25.

Three smoke trials per new provider found Swisscows successful throughout; Qwant succeeded once and was blocked twice; Yep returned valid empty responses; Dogpile, Ecosia and Mojeek were blocked. Swisscows and Qwant have real success fixtures. Ecosia success selectors remain provisional, and synthetic fixtures for other blocked providers do not establish live reliability.

Artifacts:

- [Swisscows live results](../results/swisscows-random-h2-20260911/summary.json), [manual quality](../results/swisscows-random-h2-20260911/quality.json), [version-aware ranking replay](../results/swisscows-random-h2-20260911/ranking-version-quality.json).
- [Pinned original-provider comparison](../results/retrieval-final-pinned-20260911/summary.json).
- [New-provider smoke results](../results/providers-random-h2-20260911/summary.json).

The interrupted unbounded retrieval directory is exploratory only. The earlier retrieval-budget3-random-h2 batch overlapped a binary rebuild and its aggregate is invalid; its VALIDITY.md records the limitation. The final pinned comparison supersedes it.

## Fetch concurrency and reusable clients

The [final concurrency comparison](../results/concurrency-final-20260911/summary.json) alternated 5 and 10 workers over three rounds of ten queries using Swisscows, hybrid ranking, a 3-second search budget, a 2-second fetch budget and no Kestrel cache. Concurrency 5 measured median 1.674 s / p95 3.080 s; concurrency 10 measured 1.456 s / p95 2.814 s. Both returned 150/150 slots with no failed calls and 92/150 returned slots containing fetched content. This is a roughly 13% lower observed median, not an isolated causal estimate or a scored quality comparison. Equal content counts do not establish equivalent relevance.

A [three-call reusable-client sample](reuse-client.jsonl) for the PostgreSQL query took 228, 117 and 71 ms and returned 20 candidates each time. Server caching and network variation confound these timings. The deterministic local socket tests directly establish pooling; this tiny live sample does not quantify the latency benefit independently.

## Validation and remaining evaluation

All 63 Rust tests pass, including real local HTTP/2 multiplexing/clone reuse for both HTTP clients and HTTP/1.1 socket reuse. Clippy passes with warnings denied. Three Python scorer tests pass. Original handoff hashes and whitespace checks pass.

The [30-question coding pilot](../coding-pilot-20260911/README.md) contains source snapshots, release evidence, answer keys and a repository-disjoint development/held-out split. It is a narrow release-change pilot, not a completed agent evaluation or broad code-compatibility suite. Agent-condition trials, independent scoring, indexing checks and broader candidate/cache/budget ablations remain pending. The quality gate is not met, so new engines and ranking remain opt-in. Follow-up priorities are reliable provider access, live validation of provisional adapters, the wrong-entity failure, and held-out agent outcomes before default promotion.
