# Kestrel search performance and relevance investigation

Start here in a new session. These measurements were collected in the local
`codex-otel` project on 2026-09-11 and copied into this repository for investigation.
The goal is to improve Kestrel's result quality and latency, especially the
apparent loss of multiword-query intent and domain restrictions.

## Evidence

| Configuration | Searches | Median | p95 | Direct relevance precision@5 |
|---|---:|---:|---:|---:|
| Built-in web search | 50 | 1.613s | 2.783s | 81.2% |
| Kestrel, no fetching or BM25 | 50 | 0.546s | 0.662s | 4.8% |
| Kestrel, fetching + BM25 | 30 | 1.472s | 2.439s | 10.0% |

Kestrel version: **1.1.1**. Both Kestrel configurations used fanout mode,
provider quorum 1, default DuckDuckGo/Bing/Yahoo engines, and top-k 5.
All retained Kestrel results observed in these runs came from Bing.
The initial comparison used 10 distinct queries × 5 rounds per tool.
The fetch/BM25 follow-up used 10 queries × 3 rounds and reused the saved built-in
baseline, at the user's request.

Notable failures:

- `What is the capital of Australia?` returned Capital FM radio pages.
- `why is the sky blue Rayleigh scattering` returned dictionary entries for
  “why” and music videos.
- `site:postgresql.org EXPLAIN ANALYZE BUFFERS` returned definitions of “explain”.
- `site:grafana.com Tempo TraceQL parent child spans` returned music-tempo pages.
- `Rust E0382 use of moved value how to fix` returned Rust game and generic
  programming-language pages.
- `James Webb TRAPPIST-1 b atmosphere observations 2025` returned the band James;
  with fetching/BM25, its discography was the sole retained result.
- The museum-hours query sometimes worked: the no-fetch batch had one empty
  response, one generic-London result set, then three useful result sets.

In the no-fetch batch, none of the 75 top-five slots for the three domain-filtered
queries respected the requested domain. With fetching/BM25, 3 of 45 did.
The subprocess received the complete query as **one argument**, not a shell-split
string. These observations suggest a query handling, provider, parsing, fallback,
or environmental issue; they do not establish which component is responsible.
Do not assume that disabling BM25 alone explains these failures.

## Files

- `queries.json`: the exact 10 query strings and their intended information needs.
- `results.json`: original 100 calls, raw tool output, normalized top-five
  results, query IDs, rounds, timestamps, and complete-call latency.
- `judgments.json`: explicit relevance score and rationale per query/URL.
- `summary.json` and `REPORT.md`: initial metrics and methodology.
- `fetch-bm25/`: equivalent files for the 30-call follow-up.
- Both `analyze.py` files recompute metrics from saved results and judgments.
- Both `run-kestrel.py` files accept one query argument and return JSON with
  stdout, stderr, exit status, and monotonic full-process timing.
- `COPY-MANIFEST.json`: hashes verifying the original 12 files were copied intact.

## Reproduce and compare

From the repository root:

```sh
python3 benchmarks/codex-search-2026-09-11/analyze.py
python3 benchmarks/codex-search-2026-09-11/fetch-bm25/analyze.py

python3 benchmarks/codex-search-2026-09-11/run-kestrel.py \
  'site:postgresql.org EXPLAIN ANALYZE BUFFERS'

python3 benchmarks/codex-search-2026-09-11/fetch-bm25/run-kestrel.py \
  'site:postgresql.org EXPLAIN ANALYZE BUFFERS'
```

The runners invoke `kestrel` from PATH. When testing repository changes, ensure
PATH resolves to the newly built binary, or run that binary explicitly. Record
the version and commit alongside every new batch.

**Fetch concurrency:** the completed fetch/BM25 batch used **5**. After it
finished, the user requested **10**, so its runner now explicitly passes
`--concurrency 10` for future runs. No saved results measure concurrency 10.
To reproduce the historical fetching configuration exactly:

```sh
kestrel search 'site:postgresql.org EXPLAIN ANALYZE BUFFERS' \
  --mode fanout --provider-quorum 1 --fetch --rank -k 5 \
  --content-limit 2000 --fetch-budget 20 --concurrency 5 --output json
```

Keep new results in a separate directory so these baselines stay reviewable.
Older reports mention paths relative to the original `codex-otel` repository;
the commands above are the correct paths for this copy.

## Suggested investigation order

1. Trace a complete multiword query from CLI argument through URL encoding and
   provider request construction. Include a `site:` query and punctuation.
2. Inspect provider responses and parsed result links/titles/snippets. Determine
   whether the wrong intent originates upstream or in parsing/fallback logic.
3. Compare each provider independently with fanout/quorum 1. A fast nonempty
   response is not necessarily a useful response; record the provider that won.
4. Once retrieval is correct, compare ranking off/on and fetching concurrency
   5/10 using the same queries. Track fetch failures, dropped candidates, and
   the reduction in returned results as well as duration.
5. Rerun three rounds per configuration, one query at a time, and compare
   per-query relevance and latency. Add regression tests for any confirmed bug.

## Interpretation limits

This is a small, descriptive local benchmark. Quality was scored by one assistant,
not blind independent reviewers: 0 = off-topic/generic, 1 = partial/background or
date mismatch, 2 = directly relevant. Precision@5 counts only score 2; missing
slots count as zero. Duplicate query/URL pairs reuse a judgment. Review the
judgments and revise them explicitly if necessary. Relevance is not verified
factual accuracy. Translated Codex results, old PDFs and duplicate docs are
additional usability problems in the built-in results.

The built-in tool returns more than five results and richer excerpts; only the
first five displayed results are scored, but its timing includes the complete
response. Its fetching, ranking and caching are opaque. Kestrel process time and
complete-call time are both recorded; command-launch spans alone undercount
background execution. The recorded complete-call times include automatic polling
and invocation overhead, but not model thinking between calls. Upstream caches,
region and network conditions were not controlled. Saved baselines and the
follow-up were collected at different times.

## Next benchmark: fresh coding questions requiring search

The user wants to evaluate coding-related agentic search using fresh information,
so that an LLM cannot conceal poor retrieval by answering from its own knowledge.
Use the existing ten queries as retrieval regression cases, then add a separate
dataset for this purpose. The recommendations below were researched on 2026-09-11;
check dataset release dates again when starting a new evaluation.

### Public datasets and their fit

| Dataset | Useful properties | Limitations |
|---|---|---|
| [FreshStack](https://fresh-stack.github.io/) | Developer questions paired with code/documentation and judgments about which documents support the facts needed to answer. Covers LangChain, Angular, Laravel, Godot, and YOLO. | Primarily corpus retrieval, not a ready-made live-web agent benchmark. The published October 2024 snapshot is no longer fresh. Live-web evaluation requires mapping or rejudging retrieved pages. |
| [LiveBrowseComp](https://huggingface.co/datasets/Forival/LiveBrowseComp) | 335 multi-step search questions built from obscure facts published within 90 days before dataset construction. The authors reported below 2% closed-book accuracy for their tested models. | Broad topics rather than coding, although seed sources include CVE/NVD. Freshness is relative to construction time and the tested model, not guaranteed for future models. |
| [FreshQA](https://github.com/freshllms/freshqa) | Time-sensitive questions and an established answer-refresh process. Useful for evaluating changing facts. | General knowledge rather than coding. At research time the repository linked an April 21, 2026 edition; do not assume its stated update schedule means the available snapshot is current. |

Sources and implementation references:

- [FreshStack construction and evaluation code](https://github.com/fresh-stack/freshstack):
  supports collecting GitHub documentation and community questions, with nugget
  coverage, recall and alpha-nDCG evaluation. Check dataset licenses before reuse.
- [LiveBrowseComp paper](https://arxiv.org/html/2605.28721v1): discusses dependence
  on intrinsic model knowledge, closed-book baselines, and evidence-grounding
  diagnostics. Its reported low closed-book accuracy is an experimental result,
  not a guarantee against memorization by another model.
- [Temporal drift study on FreshStack](https://arxiv.org/abs/2603.04532): motivates
  preserving dated corpus snapshots and checking whether judgments remain valid.

Recommended direction: use FreshStack's evaluation approach and construction
tools as a starting point, plus a small rolling dataset of fresh coding questions.
Do not label an adapted live-web evaluation as an official FreshStack score.

### Construct a rolling coding dataset

Source questions from releases, merged bug fixes, migration guides and
documentation changes published in the preceding 30–90 days. Prefer facts newer
than the evaluated model's documented knowledge cutoff where available, but
still measure closed-book performance; recency alone does not establish novelty.
Require an explicit version and an as-of date instead of ambiguous “latest”.

Suggested question templates (not existing benchmark questions):

- Which release first fixed issue X, and what workaround applies to the preceding
  release?
- In version X.Y, what replaced a deprecated API, and how must a supplied code
  example change?
- Which dependency versions satisfy compatibility requirements introduced in
  release X, according to the release notes and package metadata?

For each item, retain its ID, question, as-of date, package/repository and version,
expected answer, required atomic facts, gold URLs, supporting passages, source
publication dates, and archived source snapshots or immutable commit references.
Validate each answer against primary sources. Keep answer keys out of agent
context and separate development cases from held-out evaluation cases.
Check that relevant pages are discoverable; report indexing gaps separately from
query-handling failures. Preserve old dataset snapshots when refreshing answers.

### Agent evaluation protocol

1. Evaluate the same model in three conditions: no tools, built-in search, and
   Kestrel. Use a fresh isolated context for every question and condition so
   answers or retrieved evidence do not carry over between trials.
2. Hold model version, prompt, sampling settings, maximum turns, search/fetch
   budgets, and final answer format fixed. Standardize returned fields and
   evidence budgets as far as the interfaces permit. Swap only the search
   backend and disclose remaining interface differences.
3. Use three rounds per configuration unless the user requests otherwise.
   Alternate condition order and record cache state where controllable.
   A new paired evaluation should refresh both search baselines; reusing the
   historical baseline is acceptable only as a clearly labeled comparison.
4. Report all-question results and a separate breakdown for questions answered
   correctly without tools. Also report performance on questions the model
   failed without tools. Treat this as a diagnostic of retrieval dependence,
   not proof of training-data contamination. Do not silently discard easy cases.
5. Require answer citations to supporting passages actually retrieved during that
   run. Verify that each citation supports the associated claim. A correct answer
   without supporting retrieved evidence fails the grounding criterion, while
   answer correctness is still recorded separately. Tool use alone is not proof
   that the answer depended on search.
6. Preserve complete tool trajectories: queries, provider, returned URLs, fetched
   text, timestamps, errors and final answers. Prevent access to benchmark answer
   keys or published solutions, following each dataset's evaluation guidance.

### Metrics to prioritize

Evaluate retrieval and the final answer separately so model knowledge cannot
compensate for irrelevant search results.

- Retrieval: precision@5, coverage of required facts by retrieved evidence,
  version/date correctness, domain compliance, duplicate rate, and empty/error
  rate. Use gold evidence and adjudicated judgments rather than keyword matching.
- Agent outcome: answer correctness, citation support, completeness, and absolute
  accuracy improvement over the no-tools baseline. Report the closed-book-failed
  subset separately.
- Latency/cost: median and p95 complete-search latency, time until retrieved
  evidence covers all required facts, total task latency, tool calls, fetched
  bytes, and token usage where available. Define evidence sufficiency against
  the saved fact checklist rather than the agent's own claim that it is done.

Use a consistent rubric with blinded or independent review where practical.
For coding outputs, add execution checks in the specified package versions when
feasible. Keep the existing manual relevance scores as historical observations;
they are not a substitute for this stronger evidence-based evaluation.
