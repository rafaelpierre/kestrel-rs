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

The default `--query-syntax portable` uses the same lexical contract for **all
nine providers**. Constraints are checked against each
result's title and snippet **before quorum, merging, fetching and ranking**.
The complete original query is still encoded and sent as a retrieval hint; we do
not assume a provider honors its operators. This does not repair upstream
retrieval, and a provider that ignores the hint may supply no acceptable results.

```sh
# Shell single quotes preserve the double quotes sent to Kestrel.
kestrel search '"machine learning"' --engine swisscows --no-fetch --no-rank
# Both terms, anywhere in the title/snippet, without requiring adjacency.
kestrel search 'machine AND learning' --engine swisscows
kestrel search '("machine learning" OR "deep learning") -jobs site:example.com'
# Provider-specific operators and previous passthrough behavior.
kestrel search 'filetype:pdf "machine learning"' --query-syntax native
```

| Portable syntax | Metadata constraint |
|---|---|
| `machine learning`, `machine AND learning` | Both terms; they may appear in different fields |
| `"machine learning"` | Adjacent words in order within one field |
| `machine OR learning` | At least one branch |
| `NOT jobs`, `-jobs` | Exclude results whose title or snippet matches `jobs` |
| `(a OR b) c` | Grouping; otherwise NOT binds before AND, then OR |
| `site:example.com` | URL hostname equals example.com or a subdomain |
| `-site:example.com`, multiple sites with OR | Boolean hostname restrictions |

Operators are uppercase; lowercase `and`, `or`, `not` are literal terms.
Matching ignores case and collapses whitespace. Punctuation otherwise remains
literal, and identifier boundaries preserve `C++`, `C#` and underscores. Phrase
matches cannot cross the title/snippet boundary. Inside quotes, `\"` and `\\`
escape a quote and backslash. Apostrophes are ordinary characters. Unbalanced or
empty quotes, incomplete expressions, unsupported operators, wildcards, pipes,
and site URLs/paths are rejected before provider requests. Portable queries are
limited to 8192 bytes, 128 tokens and 32 levels of grouping/negation.

**Evidence limits:** these are title/snippet constraints, not semantic relevance
or a guarantee about the complete page. Missing positive-match evidence excludes
a result even when a full page might match. NOT means no match in the available
metadata, not proof of absence in the page. Missing/failed/truncated page fetches
do not alter the decision; `--no-rank` and `--no-fetch` use the same checks. An
empty accepted set does not prove that the web has no matching pages. Result URL
text and fetched body text do not supply positive keyword/phrase evidence.

Rejected results do not satisfy provider quorum. Remaining providers continue
within the existing search budget; rejection triggers no unbounded extra calls.
Diagnostics retain raw and accepted counts, `filtered_count`, and
`filtered_empty`. Original provider ranks and query provenance are retained.

| Provider | Request field | Portable checks |
|---|---|---|
| DuckDuckGo | form `q` | Shared contract above |
| Bing | URL `q` | Shared contract above |
| Yahoo | URL `p` | Shared contract above |
| Dogpile | JSON `q` | Shared contract above |
| Ecosia | URL `q` | Shared contract above |
| Swisscows | URL `query` | Shared contract above |
| Yep | URL `query` | Shared contract above |
| Qwant | URL `q` | Shared contract above |
| Mojeek | URL `q` | Shared contract above |

`--query-syntax native` preserves provider-specific query passthrough. It disables
portable parsing and metadata checks; the existing conservative standalone
positive `site:hostname` filter and HTTP(S) URL validation remain. Native Boolean,
phrase and other operator support is provider-dependent and is not guaranteed
by serialization tests. Use this mode for `filetype:`, `intitle:`, site paths,
wildcards, or other provider syntax outside the portable subset.

Rust callers can set `SearchOptions.query_syntax` to `QuerySyntax::Native` to
retain the previous contract. `SearchOptions::default()` and the single-provider
`search`/`search_blocking` APIs now use portable semantics; use `search_many` with
one engine to choose native syntax. Explicit `SearchOptions` struct literals
must add the new field or use `..Default::default()`.

The opt-in `benchmarks/query_semantics.py` runs the CLI matrix across all providers
and fetch/rank modes. Live failures and empty output are separate outcomes, not
proof of operator support. Deterministic parser, request encoding and quorum tests
run in CI without live provider availability.

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
  providers. CLI fanout defaults to five seconds; `--no-search-budget` disables
  that deadline. It is separate from `--fetch-budget`.
- `--ranking-policy provider|snippet|body|hybrid|rrf` selects opt-in policies.
  Body retains legacy BM25 behavior and requires fetching. Snippet and hybrid
  retain candidates with missing content; hybrid ignores the synthetic Source
  prefix. RRF fuses original provider ranks. Experimental scores do not replace
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
