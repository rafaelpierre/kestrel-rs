# Search providers and query semantics

New opt-in providers are tracked in [issue #6](https://github.com/rafaelpierre/kestrel-rs/issues/6).
AOL is excluded. Defaults remain DuckDuckGo, Bing and Yahoo.

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
its parser must be confirmed against a successful live result page before promotion.

## Response size limit

All providers, including Yahoo's impersonated transport, enforce a 4 MiB
(4,194,304 byte) response limit after HTTP decompression and before text decoding
or HTML/JSON parsing. The same limit applies to HTTP error bodies. Declared
oversized responses are rejected before reading; chunked and compressed bodies
are checked incrementally before appending to the retained buffer.

An oversized response returns `KestrelError::ProviderResponseTooLarge`, including
the engine, limit and HTTP status. It is not retried. Provider diagnostics record
`response_too_large`; fallback can try the next engine, and fanout retains other
successful providers. Oversized bodies are not written to provider trace files.
This fixed provider limit is independent of page-fetch response limits. It bounds
the retained response bytes, not total process memory: transport chunks, charset
conversion, parsers and concurrent searches require additional memory.

## Query language

Kestrel transports the complete native query unchanged, using form/URL/JSON
encoding appropriate to each endpoint. It does not convert quotes, lowercase
Boolean operators, split `C++`, or reinterpret arbitrary syntax as another
provider's query language.

- [Ecosia](https://support.ecosia.org/article/447-search-features) documents quoted
  phrases, `AND`, `OR`, `-term`, `site:` and provider-dependent `filetype:`.
- [Swisscows](https://support.swisscows.com/swisscows-search/search-operators/)
  documents `site:`, quoted phrases, `+`, `-`, uppercase `AND`/`OR`/`NOT`,
  `ext:`, `filetype:`, `inbody:`, `intitle:`, `inpage:`, `lang:` and `loc:`.
  Its operators are explicitly experimental.
- [Mojeek](https://www.mojeek.com/support/search-operators.html) documents
  `site:`, `inanchor:`, `intext:`, `intitle:`, `inurl:`, their `all` forms,
  and date operators. Its [operator guide](https://blog.mojeek.com/2023/08/mojeek-operators-a-guide.html)
  also documents exclusions. Date filters refer to modification dates.
- Qwant, Dogpile and Yep native syntax is passed through, but operator support
  has not been established by this implementation. Do not claim cross-provider
  Boolean/phrase parity from encoding tests alone.

All engines additionally enforce an unambiguous, standalone positive
`site:hostname` locally before quorum acceptance. Hostnames include subdomains,
not unrelated names containing the domain. Quoted literal operators, `NOT`,
`OR`, parentheses, multiple site operators and path-based site filters are left
to the provider. They are not silently treated as simple hostname restrictions.

No local keyword-overlap threshold claims to prove relevance. A valid nonempty
response still may be off-topic; quality judgments remain a separate benchmark.

## Random headers and pooled transport

Every new search/fetch client randomly selects a coherent Chrome/Firefox desktop
profile, OS and English language preference using the existing `primp` profile
catalogue. The profile stays fixed across that client's requests and retries.
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
  providers. It is separate from `--fetch-budget`.
- `--ranking-policy provider|snippet|body|hybrid|rrf` selects opt-in policies.
  Body retains legacy BM25 behavior and requires fetching. Snippet and hybrid
  retain candidates with missing content; hybrid ignores the synthetic Source
  prefix. RRF fuses original provider ranks. Experimental scores do not replace
  the public content-only `bm25_score` field.
- `KESTRELSEARCH_PROVIDER_TRACE_DIR` captures response bodies, final URL, status,
  negotiated HTTP version and generated browser profiles. Treat traces as local
  debugging artifacts; they can contain query text and provider identifiers.
- Existing benchmark artifact variables capture candidates, stage counts and
  provider outcomes, including cancellations and deadline expiry.

`SearchOptions` adds optional `search_budget`; external Rust struct literals must
add it or use `..Default::default()`. New `Engine` variants also require updating
exhaustive external matches. Normal CLI result JSON keeps its existing shape.

Sources for frontend wire-format inspection: live Swisscows JavaScript/responses,
Qwant initial props, and the independently maintained
[SearXNG adapters](https://github.com/searxng/searxng/tree/master/searx/engines).
Synthetic fixtures test contracts; they do not establish live success.
