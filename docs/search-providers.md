# Search providers and query semantics

Adapter validation is tracked in [issue #6](https://github.com/rafaelpierre/kestrel-rs/issues/6).
AOL is excluded. CLI and library search defaults are DuckDuckGo, Bing, Yahoo,
Dogpile, Ecosia, Swisscows, Yep, Qwant and Mojeek, in that order. Explicit engine
selections replace the list; `-e duckduckgo -e bing -e yahoo` restores the previous
set. This expands request fanout within existing concurrency and budget limits;
it does not establish a quality, latency or availability improvement. Failed or
unsupported-filter providers retain results from successful providers.

```sh
kestrel search 'site:postgresql.org EXPLAIN ANALYZE BUFFERS' --engine swisscows --no-fetch --no-rank --search-budget 3 --output json
kestrel search 'Rust E0382 use of moved value' --engine swisscows --engine bing --mode fanout --search-budget 3 --ranking-policy hybrid --fetch-budget 2
```

## Adapter contracts

| Engine | Browser entry point | Retrieval and parsing | Filters implemented |
|---|---|---|---|
| Dogpile | `https://www.dogpile.com/serp?q=QUERY` | POST web JSON endpoint `/api/search`; `results` entries with `clickUrl`, `title`, `description` | Native query passed unchanged; explicit region/recency flags rejected |
| Ecosia | `https://www.ecosia.org/search?q=QUERY` | HTML organic result containers; excludes recognized ad containers | Native query passed unchanged; explicit region/recency flags rejected |
| Swisscows | `https://swisscows.com/en/web?query=QUERY` | Public frontend `/v5/web/search` endpoint; decode transport envelope, retain only `WebPage` items | Country-language → language-COUNTRY locale; `uk-en` → `en-GB`; day/week/month/year freshness; spell correction disabled |
| Yep | `https://yep.com/web?q=QUERY` | Frontend `api.yep.com/search` endpoint; `query` parameter and result array in response element 1 | Native query passed unchanged; explicit region/recency flags rejected |
| Qwant | `https://www.qwant.com/?q=QUERY&t=web` | Frontend `/v3/search/web`; only `web` rows from `data.result.items.mainline`; ads excluded | Country-language → language_COUNTRY locale; recency flag rejected |
| Mojeek | `https://www.mojeek.com/search?q=QUERY` | Organic `ul.results-standard` HTML results, destination links, heading and snippet | `since` day/month/year; week mapped to an explicit UTC date seven days earlier; region flag rejected |

These are website adapters, not paid API integrations. Provider changes can break
undocumented frontend endpoints. JSON schemas are validated; challenges, unknown
HTML and malformed responses are errors, not fabricated empty results.

On September 11, 2026, the Swisscows endpoint returned PostgreSQL documentation.
Dogpile, Ecosia and Mojeek returned access blocks from this environment. Qwant succeeded in one of three randomized-header smoke trials; the other two were blocked.
Yep varied between an access block and a valid empty response. Those observations
are not availability guarantees. Ecosia's success markup fixture is provisional;
its parser still needs confirmation against a successful live result page.

## Response size limit

All providers, including Yahoo's impersonated transport, enforce a 4 MiB
(4,194,304 byte) response limit after HTTP decompression and before text decoding
or HTML/JSON parsing. The same limit applies to HTTP error bodies. Declared
oversized responses are rejected before reading; chunked and compressed bodies
are checked incrementally before appending to the retained buffer.

An oversized response returns `KestrelError::ProviderResponseTooLarge`, including
the engine, limit and HTTP status. It is not retried. Provider diagnostics record
`response_too_large`; fanout retains results from other successful providers.
Oversized bodies are not written to provider trace files.
This fixed provider limit is independent of page-fetch response limits. It bounds
the retained response bytes, not total process memory: transport chunks, charset
conversion, parsers and concurrent searches require additional memory.

## Query language

Write keyword-based full-text search (FTS) queries. Agents must NEVER submit
conversational questions or semantic prompts. Translate the request into lexical
terms while preserving its constraints: `Rayleigh scattering blue sky` expresses
the search intent of "why is the sky blue". Keep entities, technical identifiers,
requested phrases, negation, domains, versions and dates. FTS is agent input
formulation guidance; it adds no runtime rewriting, validation or query parser.

Queries are sent to providers unchanged. In `kestrel search "machine learning"`,
shell quotes group one argument; no local AND constraint is inferred.
`kestrel search '"machine learning"'` sends literal phrase quotes to providers.
Phrase, Boolean and other operator support remains provider-dependent. Kestrel
no longer parses a portable expression or rejects title/snippet term mismatches.

HTTP(S) URL validation and conservative standalone positive `site:hostname`
restrictions remain. Ranking and the opt-in BM25 fetch threshold handle relevance.

### Migration

Portable mode and `--query-syntax` are removed (both former CLI values now produce
usage status 2). Remove the flag from saved commands. Rust callers must remove
`QuerySyntax` imports and `SearchOptions.query_syntax`; all search APIs use query
passthrough. There is no replacement local Boolean/phrase filter. The optional
`--min-fetch-score` uses tokenized query text and no longer needs a syntax option;
its scores no longer exclude Boolean/site/exclusion terms through a parser.

Historical portable experiments describe previous releases, not current behavior.
The query smoke matrix now exercises passthrough and labels it explicitly.

## Random headers and pooled transport

Every new search/fetch client randomly selects one of exactly two coherent
profiles: Chrome 146 on macOS or Firefox 146 on Windows. An English language
preference (US or GB) is selected independently using the existing `primp`
profile catalogue. Chrome 148, Linux and other browser/OS pairings are excluded. The profile stays fixed across that client's requests and retries.
Provider-specific Origin, Referer, JSON Accept and content type take precedence.
Yahoo also uses the selected browser's TLS and HTTP/2 impersonation profile.

Search and fetch clients negotiate HTTP/2 through TLS ALPN and permit HTTP/1.1
fallback. The shared transport policy retains up to two idle connections per host
for five minutes, uses bounded receive windows and caches DNS answers. TCP
keepalive is 60 seconds; HTTP/2 PING is opt-in. See [HTTP/2 tuning](http2.md)
for configurable settings, warm-up and diagnostics.
Connection/Keep-Alive headers are omitted because HTTP/2 forbids them.

`KestrelClient` and its clones share their existing connection pools. Use it for
repeated agent calls, or use CLI `-q` for several queries in a single process.
Pools cannot survive separate CLI processes. `examples/reuse_benchmark.rs`
measures three searches through one client without confusing this with startup.

## Diagnostics and experiments

- `--search-budget SECS` covers provider queueing and retries, retaining completed
  providers. CLI fanout defaults to five seconds; `--no-search-budget` disables
  that deadline. It is separate from `--fetch-budget`.
- `--ranking-policy provider|snippet|body|hybrid|rrf` overrides the default lexical hybrid policy.
  Body uses positive-IDF content BM25 and requires fetching. Snippet and hybrid
  retain candidates with missing content; hybrid ignores the synthetic Source
  prefix. RRF fuses original provider ranks without lexical tokenization or BM25
  scoring. Snippet/hybrid and the metadata fetch threshold precompute query-term
  statistics once per corpus; scores and stable ties are preserved. Hybrid also ranks metadata with `--no-fetch`; `--no-rank` preserves candidate order. Composite scores do not replace
  the public content-only `bm25_score` field.
- `KESTRELSEARCH_PROVIDER_TRACE_DIR` captures response bodies, final URL, status,
  negotiated HTTP version and generated browser profiles. Treat traces as local
  debugging artifacts; they can contain query text and provider identifiers.
  See [provider diagnostic records](provider-diagnostics.md) for attempt IDs,
  status/challenge semantics, censored timings and cancellation accounting.
- Existing benchmark artifact variables capture candidates, stage counts and
  provider outcomes, including cancellations and deadline expiry.

`SearchOptions` adds optional `search_budget`; external Rust struct literals must
add it or use `..Default::default()`. New `Engine` variants also require updating
exhaustive external matches. Normal CLI result JSON keeps its existing shape.

Sources for frontend wire-format inspection: live Swisscows JavaScript/responses,
Qwant initial props, and the independently maintained
[SearXNG adapters](https://github.com/searxng/searxng/tree/master/searx/engines).
Synthetic fixtures test contracts; they do not establish live success.

## Completed-response parsing (#116)

Completed DuckDuckGo, Bing and Yahoo validation/extraction now share one HTML
DOM. Ecosia and Mojeek also reuse the document supplied by the completed-response
dispatcher. Constant selectors are compiled once per parsing thread and reused.
The four JSON adapters bypass HTML document construction; markup inside result
fields is cleaned as result text, not interpreted as a page-level challenge.
Qwant's top-level challenge URL is still rejected. HTML HTTP error bodies retain
HTML challenge diagnostics even for providers normally returning JSON.

Dogpile, Yep and Qwant each decode one JSON representation in their completed
adapter. Swisscows decodes its outer JSON plus the encoded payload when present;
these are distinct wire representations. HTML fragments in JSON titles/snippets
still require text extraction. Streaming frame recognition, ranks, metadata and
result-count stopping remain unchanged.

Transport challenge diagnostics and completed extraction now share one worker-local
HTML/JSON representation, including EOF fallbacks. The same bounded worker both
classifies and extracts; only owned results leave it, and the DOM is dropped before
its capacity is released. HTTP error bodies are classified without extracting
results. Extraction failures after successful HTTP responses do not cause retries.
The broader diagnostic markers and narrower adapter-specific rejection rules remain
separate decisions over the shared document.

Incremental JSON snapshots share their parsed value between challenge detection
and extraction (removing Qwant's duplicate snapshot decode). Array-path probes
are synthetic prefixes with a sentinel; snapshots contain closed items plus
synthetic closing delimiters; EOF contains the complete response. These are distinct
inputs and are still validated independently. Reusing a partial snapshot at EOF
would skip validation of the trailing response. This change does not replace
streaming framing, change incremental acceptance, or remove its existing bounded
probe/snapshot passes. See the [nine-adapter inventory and validation methodology](provider-parse-reuse.md).
Worker capacity and cancellation belong to #117 / PR #159; #16 still requires
its complete cross-slice acceptance evidence.

## BM25 score compatibility

Content-only BM25 and optional pre-ranking use the `bm25` crate with positive IDF `ln(1 + (N - df + 0.5) / (df + 0.5))`, k1=1.5 and b=0.75. Matching terms remain positive even in half or all documents. Kestrel preserves its tokenization, query grouping and stable ties; titles/snippets are not added to default body ranking. Scores are computed in f32 and exposed as JSON numbers/f64, so values and ordering can differ from older releases. Scores are relative to the candidate pool, not calibrated relevance probabilities. Experimental snippet/hybrid scoring and the optional fetch-score threshold retain their existing f64 implementation.

## Shared HTTP retry policy (#205)

Standard and Yahoo transports share completed-response classification/extraction,
status retry decisions, Retry-After/backoff selection and bounded streaming body
processing. Backend adapters retain their own send-error classification: Yahoo
retries all send errors, while standard HTTP retries timeout/connect/request
errors only. Both allow at most three application sends per provider attempt.
HTTP 408/429/5xx retries stop on detected non-2xx challenges; successful-status
body failures, oversized bodies and extraction errors are terminal. Other
error-status body failures retain the status-based retry decision and typed body
diagnostics. Valid Retry-After guidance is honored up to 15 seconds; longer
waits stop the request and invalid values use bounded jittered backoff.

This is a behavior-preserving internal refactor with no new dependencies or
public API/CLI/schema changes. Streaming publication, cancellation, parse-once
ownership and the decompressed response-size limit remain unchanged.
