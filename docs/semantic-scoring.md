# Semantic scoring building blocks

`ranking::semantic` provides backend-independent library primitives for #112.
It supplies no production semantic backend, model download, automatic fallback,
CLI mode, inference cache storage, or change to search/fetch behavior. Production
selection depends on #109; integration and deadlines/fallback configuration belong
to #113/#114. Existing `--ranking-policy hybrid` remains lexical BM25 over title,
snippet and available body. Provider fusion (#8) is a separate stage.

`SemanticScorer` reports immutable backend/model/configuration identity and async
availability, then scores query/text pairs. `score_batch` enforces explicit pair,
per-query/text UTF-8 byte, total byte and nonzero time limits before invocation.
It never truncates text. The deadline includes availability and inference. Empty
batches do not touch the backend. Each input ID must appear exactly once in output;
backend order is irrelevant. None means unavailable evidence; finite scores use
higher-is-better ordering. Missing, extra, duplicate IDs or nonfinite scores fail
the entire batch. Errors remain typed, with no implicit fallback or retry.
Dropping the future cancels owned work: implementations must yield and must not
leave detached inference running. This contract cannot forcibly stop a backend
that blocks or violates cancellation. Future adapters need their own resource
and worker cancellation validation.

`cache_key` hashes versioned, unambiguous serialized exact query/text bytes plus
backend, immutable model revision and configuration. Configuration must include
text selection, truncation/chunking and score interpretation. IDs do not affect
identity. Do not cache failures; no cache is initialized by these primitives.

`combine` accepts an ordered candidate list, query count and explicit
query-index/candidate-index evidence. Repeated query strings remain independent
indices; callers must supply every desired query association, including shared
URLs, instead of using only representative `SearchResult.query`. Out-of-range
indices and unparseable URLs fail. URL aliases use existing search canonicalization. Duplicate rows or
aliases contribute the best finite score per component, never repeated votes.

For each query, each component ranks finite scores descending (zero and negative
are valid), breaking ties by canonical URL and assigning ordinal ranks starting
at one. Missing/nonfinite scores have no rank and contribute zero. Equal-weight
RRF is `sum(1 / (60 + rank))`; raw lexical and semantic magnitudes are never added.
Final ties use canonical URL. Query lists are interleaved in input query order,
emitting each canonical URL once; candidates without any rows append in canonical
URL order. This primitive does not cap results. Candidates with no valid scores
are retained, as are semantic-only and exact-term-only candidates.

The first URL representative preserves all result fields, including lexical
`bm25_score`; distinct source occurrences from aliases are merged. `CombinedRank`
provides separate per-query component ranks and combined scores. It does not
relabel lexical scores. Existing BM25 zero filtering and experimental policies
are untouched. Callers must not apply legacy zero filtering before combination.
Input order selects metadata representatives; rank ordering for identical evidence
is independent of evidence row order. Source occurrence order follows input order.

Deterministic fixtures cover hand-calculated fusion, bounds, ties, duplicate and
shared URLs, repeated query indices, missing/invalid scores, empty inputs, batch
mapping/errors, cancellation and cache identity. No quality or speed gain is
claimed; adoption requires the separately scoped production and held-out work.
