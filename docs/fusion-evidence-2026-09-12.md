# Four-query fusion and latency evidence

**Historical measurements:** these runs predate result-minimum precedence and portable query filtering (#54). They do not measure the current implementation. Current fanout ignores provider quorum; the two-provider AND constraint below no longer applies. New benchmark runs write separate artifacts.

Live test started **2026-09-12T15:33:05.542797+00:00**. All **48/48 runs** reached at least five unique results from two providers.

The comparison is streaming early stopping versus completed-batch early stopping. **Neither arm waits for every provider.** The two-provider quorum is enabled only for this experiment to demonstrate fusion; the normal fanout default remains five unique results without a provider quorum.

## Latency

| Query | Batch median | Streaming median | Median reduction | Streaming faster in matched pairs |
| --- | ---: | ---: | ---: | ---: |
| Rust ownership borrowing documentation | 343.5 ms | 334.5 ms | 2.6% | 3/6 (1 ties) |
| PostgreSQL EXPLAIN ANALYZE documentation | 387 ms | 285.5 ms | 26.2% | 5/6 (0 ties) |
| what is machine learning | 298.5 ms | 285 ms | 4.5% | 5/6 (0 ties) |
| how do solar panels generate electricity | 315 ms | 289.5 ms | 8.1% | 4/6 (0 ties) |

Across all 24 observations per arm, median latency changed from **328 ms to 297 ms** (9.5% lower). Streaming was faster in **17/24** matched pairs, tied in 1, and slower in 6.

Nearest-rank p95 was **614 ms batch / 744 ms streaming**. The median improvement does not establish an improvement in tail latency. This is a small, sequential, live-network experiment; provider response variation remains a confounder.

| Connection setup | Batch median | Streaming median |
| --- | ---: | ---: |
| Fresh clients | 395.5 ms | 321 ms |
| Reused clients | 262 ms | 211 ms |

## Final result sets and provider inputs

Each example below is the successful streaming run closest to that query’s median streaming latency (ties broken by trial, then connection setup). No example is selected for favorable speed or results. Final results use the actual default provider round-robin fusion and canonical-URL deduplication, followed by taking the first five. Source ranks show the original provider positions.

### 1. Rust ownership borrowing documentation

Representative run: trial 2, fresh clients, **321 ms**. Retained provider records: **bing: 2**, **swisscows: 10**; **12 unique candidates**, 0 duplicate occurrences merged. **4 unfinished requests cancelled.**

| Final position | Result | Provider source / original rank |
| ---: | --- | --- |
| 1 | [Rust — Explore, Build and Survive](<https://rust.facepunch.com/>) | bing #1 |
| 2 | [How to Use Rust Ownership and Borrowing](<https://oneuptime.com/blog/post/2026-01-26-rust-ownership-borrowing-guide/view>) | swisscows #1 |
| 3 | [Rust Programming Language](<https://rust-lang.org/>) | bing #2 |
| 4 | [Understanding Ownership - The Rust Programming Language](<https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html>) | swisscows #2 |
| 5 | [Ownership and Borrowing in Rust: Complete Guide 2026 \| SharpSkill](<https://sharpskill.dev/en/blog/rust/ownership-borrowing-rust-complete-guide>) | swisscows #3 |

<details>
<summary>Retained provider inputs and their positions in the final set</summary>

**bing**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [Rust — Explore, Build and Survive](<https://rust.facepunch.com/>) | 1 |
| 2 | [Rust Programming Language](<https://rust-lang.org/>) | 3 |

**swisscows**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [How to Use Rust Ownership and Borrowing](<https://oneuptime.com/blog/post/2026-01-26-rust-ownership-borrowing-guide/view>) | 2 |
| 2 | [Understanding Ownership - The Rust Programming Language](<https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html>) | 4 |
| 3 | [Ownership and Borrowing in Rust: Complete Guide 2026 \| SharpSkill](<https://sharpskill.dev/en/blog/rust/ownership-borrowing-rust-complete-guide>) | 5 |
| 4 | [Understanding Rust Ownership and Borrowing with Examples](<https://progressivecoder.com/understanding-rust-ownership-and-borrowing-with-examples/>) | Outside final five |
| 5 | [Rust ownership and borrowing - understanding ownership in Rust](<https://www.zetcode.com/rust/ownership/>) | Outside final five |
| 6 | [Rust Tutorial => Ownership and borrowing](<https://riptutorial.com/rust/example/15353>) | Outside final five |
| 7 | [References and Borrowing](<https://doc.rust-lang.org/1.8.0/book/references-and-borrowing.html>) | Outside final five |
| 8 | [Understanding Memory Management, Part 4: Rust Ownership and Borrowing](<https://educatedguesswork.org/posts/memory-management-4/>) | Outside final five |
| 9 | [r/rust on Reddit: Most detailed account of ownership and borrowing](<https://www.reddit.com/r/rust/comments/1hjztyw/most_detailed_account_of_ownership_and_borrowing/>) | Outside final five |
| 10 | [r/rust on Reddit: I still don’t fully understand ownership and borrowing in Rust — can someone explain it simply?](<https://www.reddit.com/r/rust/comments/1pn6s2o/i_still_dont_fully_understand_ownership_and/>) | Outside final five |

</details>

<details>
<summary>Every provider outcome for this run</summary>

| Provider | Outcome | Reported record count | Contribution retained in fusion |
| --- | --- | ---: | ---: |
| bing | cancelled_min_results | 2 | 2 |
| dogpile | request_error | 0 | 0 |
| duckduckgo | cancelled_min_results | 0 | 0 |
| ecosia | request_error | 0 | 0 |
| mojeek | request_error | 0 | 0 |
| qwant | request_error | 0 | 0 |
| swisscows | results | 10 | 10 |
| yahoo | cancelled_min_results | 0 | 0 |
| yep | cancelled_min_results | 0 | 0 |

</details>

### 2. PostgreSQL EXPLAIN ANALYZE documentation

Representative run: trial 2, fresh clients, **292 ms**. Retained provider records: **bing: 2**, **swisscows: 15**; **17 unique candidates**, 0 duplicate occurrences merged. **4 unfinished requests cancelled.**

| Final position | Result | Provider source / original rank |
| ---: | --- | --- |
| 1 | [PostgreSQL : The world's most advanced open source database](<https://www.postgresql.org/>) | bing #1 |
| 2 | [PostgreSQL: Documentation: 18: EXPLAIN](<https://www.postgresql.org/docs/current/sql-explain.html>) | swisscows #1 |
| 3 | [PostgreSQL : Downloads](<https://www.postgresql.org/download/>) | bing #2 |
| 4 | [PostgreSQL: Documentation: 18: 14.1. Using EXPLAIN](<https://www.postgresql.org/docs/current/using-explain.html>) | swisscows #2 |
| 5 | [PostgreSQL: Documentation: 18: ANALYZE](<https://www.postgresql.org/docs/current/sql-analyze.html>) | swisscows #3 |

<details>
<summary>Retained provider inputs and their positions in the final set</summary>

**bing**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [PostgreSQL : The world's most advanced open source database](<https://www.postgresql.org/>) | 1 |
| 2 | [PostgreSQL : Downloads](<https://www.postgresql.org/download/>) | 3 |

**swisscows**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [PostgreSQL: Documentation: 18: EXPLAIN](<https://www.postgresql.org/docs/current/sql-explain.html>) | 2 |
| 2 | [PostgreSQL: Documentation: 18: 14.1. Using EXPLAIN](<https://www.postgresql.org/docs/current/using-explain.html>) | 4 |
| 3 | [PostgreSQL: Documentation: 18: ANALYZE](<https://www.postgresql.org/docs/current/sql-analyze.html>) | 5 |
| 4 | [The EXPLAIN query plan - AWS Prescriptive Guidance](<https://docs.aws.amazon.com/prescriptive-guidance/latest/postgresql-query-tuning/explain-query-plan.html>) | Outside final five |
| 5 | [PostgreSQL: Documentation: 9.4: Using EXPLAIN](<https://www.postgresql.org/docs/9.4/using-explain.html>) | Outside final five |
| 6 | [PostgreSQL: Documentation: 9.1: EXPLAIN](<https://www.postgresql.org/docs/9.1/sql-explain.html>) | Outside final five |
| 7 | [PostgreSQL: Documentation: 9.4: ANALYZE](<https://www.postgresql.org/docs/9.4/sql-analyze.html>) | Outside final five |
| 8 | [PostgreSQL: Documentation: 13: ANALYZE](<https://www.postgresql.org/docs/13/sql-analyze.html>) | Outside final five |
| 9 | [The Basics of Postgres Query Planning · pganalyze](<https://pganalyze.com/docs/explain/basics-of-postgres-query-planning>) | Outside final five |
| 10 | [PostgreSQL: Documentation: 10: EXPLAIN](<https://www.postgresql.org/docs/10/sql-explain.html>) | Outside final five |
| 11 | [Explaining Your Postgres Query Performance \| Crunchy Data Blog](<https://www.crunchydata.com/blog/get-started-with-explain-analyze>) | Outside final five |
| 12 | [PostgreSQL : Documentation: 18: EXPLAIN : Postgres Professional](<https://postgrespro.com/docs/postgresql/current/sql-explain>) | Outside final five |
| 13 | [PostgreSQL: Documentation: 13: EXPLAIN](<https://www.postgresql.org/docs/13/sql-explain.html>) | Outside final five |
| 14 | [PostgreSQL: Documentation: 11: EXPLAIN](<https://www.postgresql.org/docs/11/sql-explain.html>) | Outside final five |
| 15 | [Monitoring Postgres EXPLAIN plans · pganalyze](<https://pganalyze.com/docs/explain>) | Outside final five |

</details>

<details>
<summary>Every provider outcome for this run</summary>

| Provider | Outcome | Reported record count | Contribution retained in fusion |
| --- | --- | ---: | ---: |
| bing | cancelled_min_results | 2 | 2 |
| dogpile | request_error | 0 | 0 |
| duckduckgo | cancelled_min_results | 0 | 0 |
| ecosia | request_error | 0 | 0 |
| mojeek | request_error | 0 | 0 |
| qwant | request_error | 0 | 0 |
| swisscows | results | 15 | 15 |
| yahoo | cancelled_min_results | 0 | 0 |
| yep | cancelled_min_results | 0 | 0 |

</details>

### 3. what is machine learning

Representative run: trial 2, fresh clients, **311 ms**. Retained provider records: **bing: 2**, **swisscows: 8**; **9 unique candidates**, 1 duplicate occurrences merged. **4 unfinished requests cancelled.**

| Final position | Result | Provider source / original rank |
| ---: | --- | --- |
| 1 | [What is machine learning ? - IBM](<https://www.ibm.com/think/topics/machine-learning>) | bing #1, swisscows #1 |
| 2 | [Introduction to Machine Learning - GeeksforGeeks](<https://www.geeksforgeeks.org/machine-learning/introduction-machine-learning/>) | bing #2 |
| 3 | [Machine learning - Wikipedia](<https://en.wikipedia.org/wiki/Machine_learning>) | swisscows #2 |
| 4 | [What is Machine Learning? How It Works, Types and Use Cases \| Databricks Blog](<https://www.databricks.com/blog/machine-learning>) | swisscows #3 |
| 5 | [Machine Learning \| NNLM](<https://www.nnlm.gov/resources/data/data-glossary/machine-learning>) | swisscows #4 |

<details>
<summary>Retained provider inputs and their positions in the final set</summary>

**bing**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [What is machine learning ? - IBM](<https://www.ibm.com/think/topics/machine-learning>) | 1 |
| 2 | [Introduction to Machine Learning - GeeksforGeeks](<https://www.geeksforgeeks.org/machine-learning/introduction-machine-learning/>) | 2 |

**swisscows**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [What is Machine Learning? \| IBM](<https://www.ibm.com/think/topics/machine-learning>) | 1 |
| 2 | [Machine learning - Wikipedia](<https://en.wikipedia.org/wiki/Machine_learning>) | 3 |
| 3 | [What is Machine Learning? How It Works, Types and Use Cases \| Databricks Blog](<https://www.databricks.com/blog/machine-learning>) | 4 |
| 4 | [Machine Learning \| NNLM](<https://www.nnlm.gov/resources/data/data-glossary/machine-learning>) | 5 |
| 5 | [Machine learning — definition, models, and applications](<https://business.adobe.com/blog/basics/what-is-machine-learning>) | Outside final five |
| 6 | [What Is Machine Learning? Understanding ML \| Workday US](<https://blog.workday.com/en-us/what-is-machine-learning-understanding-ml.html>) | Outside final five |
| 7 | [What is machine learning? \| Microsoft Azure](<https://azure.microsoft.com/en-us/resources/cloud-computing-dictionary/what-is-machine-learning-platform/>) | Outside final five |
| 8 | [What Is Machine Learning? \| Quanta Magazine](<https://www.quantamagazine.org/what-is-machine-learning-20240708/>) | Outside final five |

</details>

<details>
<summary>Every provider outcome for this run</summary>

| Provider | Outcome | Reported record count | Contribution retained in fusion |
| --- | --- | ---: | ---: |
| bing | cancelled_min_results | 2 | 2 |
| dogpile | request_error | 0 | 0 |
| duckduckgo | cancelled_min_results | 0 | 0 |
| ecosia | request_error | 0 | 0 |
| mojeek | request_error | 0 | 0 |
| qwant | request_error | 0 | 0 |
| swisscows | results | 8 | 8 |
| yahoo | cancelled_min_results | 0 | 0 |
| yep | cancelled_min_results | 0 | 0 |

</details>

### 4. how do solar panels generate electricity

Representative run: trial 1, reused clients, **258 ms**. Retained provider records: **bing: 2**, **swisscows: 9**; **11 unique candidates**, 0 duplicate occurrences merged. **4 unfinished requests cancelled.**

| Final position | Result | Provider source / original rank |
| ---: | --- | --- |
| 1 | [DO \| English meaning - Cambridge Dictionary](<https://dictionary.cambridge.org/dictionary/english/do>) | bing #1 |
| 2 | [r/explainlikeimfive on Reddit: Eli5, How the hell do solar panels work?](<https://www.reddit.com/r/explainlikeimfive/comments/13ycao0/eli5_how_the_hell_do_solar_panels_work/>) | swisscows #1 |
| 3 | [DO Definition & Meaning - Merriam-Webster](<https://www.merriam-webster.com/dictionary/do>) | bing #2 |
| 4 | [Solar Explained—Photovoltaics and Electricity.](<https://www.eia.gov/energyexplained/solar/photovoltaics-and-electricity.php>) | swisscows #2 |
| 5 | [Solar panel - Wikipedia](<https://en.wikipedia.org/wiki/Solar_panel>) | swisscows #3 |

<details>
<summary>Retained provider inputs and their positions in the final set</summary>

**bing**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [DO \| English meaning - Cambridge Dictionary](<https://dictionary.cambridge.org/dictionary/english/do>) | 1 |
| 2 | [DO Definition & Meaning - Merriam-Webster](<https://www.merriam-webster.com/dictionary/do>) | 3 |

**swisscows**

| Provider rank | Original provider record | Final position |
| ---: | --- | ---: |
| 1 | [r/explainlikeimfive on Reddit: Eli5, How the hell do solar panels work?](<https://www.reddit.com/r/explainlikeimfive/comments/13ycao0/eli5_how_the_hell_do_solar_panels_work/>) | 2 |
| 2 | [Solar Explained—Photovoltaics and Electricity.](<https://www.eia.gov/energyexplained/solar/photovoltaics-and-electricity.php>) | 4 |
| 3 | [Solar panel - Wikipedia](<https://en.wikipedia.org/wiki/Solar_panel>) | 5 |
| 4 | [How does solar power work? \| National Grid](<https://www.nationalgrid.com/stories/energy-explained/how-does-solar-power-work>) | Outside final five |
| 5 | [Solar Energy – SEIA](<https://seia.org/initiatives/about-solar-energy/>) | Outside final five |
| 6 | [Turning sunlight into electricity: how does solar power work? \| Net Zero Economy Authority](<https://www.nzea.gov.au/turning-sunlight-electricity-how-does-solar-power-work>) | Outside final five |
| 7 | [What are Solar Panels Made of and How do they Actually Produce Electricity? \| Viridis Energy](<https://www.viridisenergy.com/learning-center/lesson/29/what-are-solar-panels-made-of-and-how-do-they-actually-produce-electricity>) | Outside final five |
| 8 | [SolarWindow Technologies — Powering the Impossible](<https://solarwindow.com/>) | Outside final five |
| 9 | [Solar cell - Wikipedia](<https://en.wikipedia.org/wiki/Solar_cell>) | Outside final five |

</details>

<details>
<summary>Every provider outcome for this run</summary>

| Provider | Outcome | Reported record count | Contribution retained in fusion |
| --- | --- | ---: | ---: |
| bing | cancelled_min_results | 2 | 2 |
| dogpile | request_error | 0 | 0 |
| duckduckgo | cancelled_min_results | 0 | 0 |
| ecosia | request_error | 0 | 0 |
| mojeek | request_error | 0 | 0 |
| qwant | request_error | 0 | 0 |
| swisscows | results | 9 | 9 |
| yahoo | cancelled_min_results | 0 | 0 |
| yep | cancelled_min_results | 0 | 0 |

</details>

## Relevance observations

The raw examples expose a separate quality problem: Bing returned the Rust video game for the ownership query and dictionary definitions of “do” for the solar-panel query. Those are structurally valid search records but off-topic. The report preserves them rather than cleaning the final sets by hand. This experiment demonstrates source fusion and observed latency changes; it does not establish that the final ranking is relevant. Error/challenge rejection alone cannot solve query relevance.

## Excluded responses and cancellation evidence

Only Bing and Swisscows contributed results in this session. All nine providers were enabled. HTTP failures, known challenges, empty responses and records excluded by URL/site filtering contributed zero. A provider marked `cancelled_min_results` can still contribute completed records received before its body was cancelled; the cancellation itself is not a result.

| Logical provider outcome, across all 48 searches | Count |
| --- | ---: |
| cancelled_min_results | 169 |
| challenge | 3 |
| request_error | 185 |
| results | 75 |

In **21 provider responses**, streaming returned complete contributing records while the body had not reached EOF. These are direct observations of stopping during download, rather than merely winning a race between completed provider responses.

## Every matched latency pair

| Query | Trial | Clients | Batch ms | Stream ms | Stream minus batch ms | Both thresholds met |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| Rust ownership borrowing documentation | 1 | Fresh | 708 | 273 | -435 | Yes |
| PostgreSQL EXPLAIN ANALYZE documentation | 1 | Fresh | 425 | 476 | +51 | Yes |
| what is machine learning | 1 | Fresh | 614 | 350 | -264 | Yes |
| how do solar panels generate electricity | 1 | Fresh | 377 | 744 | +367 | Yes |
| Rust ownership borrowing documentation | 1 | Reused | 395 | 574 | +179 | Yes |
| PostgreSQL EXPLAIN ANALYZE documentation | 1 | Reused | 380 | 302 | -78 | Yes |
| what is machine learning | 1 | Reused | 270 | 583 | +313 | Yes |
| how do solar panels generate electricity | 1 | Reused | 301 | 258 | -43 | Yes |
| Rust ownership borrowing documentation | 2 | Fresh | 321 | 321 | +0 | Yes |
| PostgreSQL EXPLAIN ANALYZE documentation | 2 | Fresh | 397 | 292 | -105 | Yes |
| what is machine learning | 2 | Fresh | 420 | 311 | -109 | Yes |
| how do solar panels generate electricity | 2 | Fresh | 447 | 487 | +40 | Yes |
| Rust ownership borrowing documentation | 2 | Reused | 269 | 783 | +514 | Yes |
| PostgreSQL EXPLAIN ANALYZE documentation | 2 | Reused | 228 | 188 | -40 | Yes |
| what is machine learning | 2 | Reused | 238 | 231 | -7 | Yes |
| how do solar panels generate electricity | 2 | Reused | 255 | 180 | -75 | Yes |
| Rust ownership borrowing documentation | 3 | Fresh | 366 | 348 | -18 | Yes |
| PostgreSQL EXPLAIN ANALYZE documentation | 3 | Fresh | 394 | 279 | -115 | Yes |
| what is machine learning | 3 | Fresh | 327 | 259 | -68 | Yes |
| how do solar panels generate electricity | 3 | Fresh | 329 | 321 | -8 | Yes |
| Rust ownership borrowing documentation | 3 | Reused | 229 | 183 | -46 | Yes |
| PostgreSQL EXPLAIN ANALYZE documentation | 3 | Reused | 249 | 191 | -58 | Yes |
| what is machine learning | 3 | Reused | 219 | 191 | -28 | Yes |
| how do solar panels generate electricity | 3 | Reused | 293 | 179 | -114 | Yes |

## Method and reproduction

- Four distinct queries; three repeats; two connection setups; two modes: 48 searches, 24 matched pairs.
- Both modes require five valid unique results and two contributing providers. Both cancel unfinished requests immediately when those conditions hold.
- Concurrency nine; three-second total search deadline; all nine adapters enabled.
- One browser/OS/language profile for the entire experiment. Fresh clients use that same profile. Reused clients have separate pools for each mode; mode order alternates by query and trial.
- Timing covers search and fusion. Page fetching, BM25/body ranking, process startup and client construction are excluded. Results shown are the provider-ranking final five, not an independently reranked set.
- Debug build on one machine/network session. No failed/slow runs are dropped. This demonstrates observed median reductions and correct fusion, not a universal or statistically established speedup.
- Inputs shown above are observed provider records whose original ranks occur in retained fused provenance. Later/unaccepted snapshots and failed-provider records are not presented as accepted inputs.

Profile: `{'accept_language': 'en-GB,en;q=0.9', 'browser': 'ChromeV146', 'os': 'Windows'}`.

```sh
cargo test --lib live_fusion_latency_evidence -- --ignored --nocapture
python3 benchmarks/fusion_evidence_report.py --input benchmarks/results/fusion-evidence-result-minimum/runs.json --output docs/fusion-evidence-result-minimum.md
```

Raw observations: `benchmarks/results/fusion-evidence/runs.json`. SHA-256: `9be5d41a5483a97d1257b4d4b915961592b64b0209e76b10a2033c05677d5ee6`.

The JSON includes every final set, source record snapshot, provider outcome, cancellation count, and timing observation. The live test is ignored by the normal test suite.
