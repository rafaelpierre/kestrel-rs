# Issue #40 — completed feasibility decision

**Decision, 2026-09-12: no-go for both Felo and Manus as Kestrel keyless HTTP
search providers.** This is a scope-specific decision, not a claim that their
products cannot search. Felo can return useful sources in a browser, but new
queries require a browser security token and anonymous use reached a guest limit.
Manus's anonymous prompt submission redirects to login; its API requires
credentials and asynchronous task orchestration.

The investigation is complete with reproducible blockers. No production adapter
or engine registration is justified. Unavailable success-quality, operator and
longitudinal measurements are explicitly identified below rather than fabricated.

## Felo: access, protocol and extraction

Public entry: <https://felo.ai/search>. We submitted
`Rust E0382 use of moved value` anonymously twice, at approximately 12:52 and
12:58 UTC. Both browser submissions completed with 11 sources. Each new tab used
the existing anonymous browser profile; these were **not isolated cookie jars**.
The page performed automatic security checking; no challenge was solved manually.

Actual frontend code identifies `POST /api-proxy/main/search/threads`, JSON
request bodies, `credentials: include`, and `cf_token` for anonymous users.
Sanitized browser console request inspection confirmed `category: google`,
`mode: concise`, `agent_lang: en`, `search_options.langcode: en-GB`,
`stream_protocol: message_center_v1`, and `enable_task_state: true`.
The complete English query was preserved in the request and persisted thread.
The token itself was neither extracted nor replayed.

A direct request using the observed fields without a browser token returned:

```http
POST https://felo.ai/api-proxy/main/search/threads
Content-Type: application/json

{"query":"Rust E0382 use of moved value","search_uuid":"<new UUID>",
 "lang":"","agent_lang":"en","search_options":{"langcode":"en-GB"},
 "search_video":true,"query_from":"default","category":"google",
 "model":"","mode":"concise","stream_protocol":"message_center_v1",
 "enable_task_state":true}

HTTP 400
{"detail":{"error_type":"turnstile_session_token_required",
           "message":"turnstile_session_token is required"}}
```

This response recurred in the second measurement window. Other attempts received
HTTP 429 with `Retry-After`. A subsequent browser query, `why is the sky blue`,
returned “You've reached the guest search limit. Log in to continue searching.”
Its UI displayed zero sources **and an error**; this is not a valid empty search.
The quota amount/reset period was not established, and the quota may have been
influenced by the direct probes. No browser profile reset or token workaround was
attempted. Browser successes were 2/3 exploratory submissions across the two
coding queries and the later general query. This tiny, unbalanced sample is not
an availability estimate. A fresh-information browser trial was not submitted
after the explicit guest limit; direct blocked trials included that query.

The frontend accepts `text/event-stream`; its parser separates `contexts` and
`final_contexts` (`data.sources`), `resources`, generated `answer` text, and
completion/error events. It also supports message-center stream recovery. These
are code-observed protocol fields, **not captured successful SSE fixtures**.
The creation request is blocked for a plain HTTP client before source events.

However, HTTP GET of the completed browser-created `/search/<thread-id>` page
succeeded without credentials. Its `__NEXT_DATA__.props.pageProps.threads` array
contained `status: completed`, `query`, `query_rewriter_info`, and
`recall_contexts` records with `link`, `title`, `snippet`, and provider order.
The extractor in `benchmarks/provider-feasibility/felo_extract.py` successfully
extracted all 11 sources from this real response. It does **not** create a query;
a completed browser-generated thread URL is a prerequisite.

The live fixture preserves the query, completion state, rewrite information and
all 11 source records. Device/visitor/thread identifiers, generated answer,
unrelated props and layout markup were removed. It is a documented projection
of a real HTTP response, not invented success data. Rank 7 is Rust's official
E0382 documentation. Top-five titles/URLs are relevant to Rust ownership, but
rank 1's expanded snippet is a cookie banner; useful-snippet coverage is 4/5 on
this single extraction. This is separate from controlled provider quality scores.
Generated answer text was Chinese despite English input and `agent_lang: en`.

Frontend sources (captured locally on September 12):

- [Felo app configuration](https://felo.ai/_next/static/chunks/pages/_app-375504f00c1df971.js), SHA-256 `d8401516b2e9c4855fe3e2f71219e089c22042f433d24daa47e29f854e7a093c`.
- [Felo request and event parser](https://felo.ai/_next/static/chunks/66123-8fe4e93e04b6929b.js), SHA-256 `744100ca5fb77ced912850d0e2985de2fb9b124179591115f921e5652eb28101`.

### Separate credentialed alternative

The [official Felo Chat API](https://openapi.felo.ai/docs/api-reference/chat)
is `POST https://openapi.felo.ai/v2/chat`, with bearer API-key authentication.
It accepts a natural-language query and documents answer generation plus
`data.resources[].link/title/snippet`. Query rewriting is part of its documented
behavior. A live request without a key returned HTTP 401, `UNAUTHORIZED`,
`API Key is required` (sanitized fixture included). This is an account/API-key
integration, even where trial credits exist. No paid or authenticated request was
made; cost, paid latency and success quality were not measured. Do not confuse
this API with the website's proxy/SSE contract.

## Manus: independent access and task contract

Public entry: <https://manus.im/>. We located the actual contenteditable prompt,
entered `Search the web for Rust E0382 use of moved value and list source URLs.`,
and submitted it. The browser navigated to
`/login?redirectUrl=%2Fapp%3Ffrom%3Dguest`. This establishes a blocker on the actual
anonymous task-submission flow, beyond simply following the Sign in link.
No source results were returned (0/1 exploratory browser prompt submission).
Further browser quality trials were stopped at this authentication boundary.

The website's public JavaScript independently exposes task/session-oriented
interfaces, including `/api/chat/getSessionV2` and `/api/chat/getSessionOutline`,
with session IDs and optional shared/private context. A shared task reader does
not provide a public arbitrary-query search endpoint.
[Inspected website bundle](https://files.manuscdn.com/webapp/_next/static/chunks/263k91sdjqrpr.js).

The separate [official API authentication guide](https://open.manus.ai/docs/v2/authentication)
requires `x-manus-api-key` or OAuth bearer authentication. Its
[task lifecycle](https://open.manus.ai/docs/v2/task-lifecycle) documents
`POST https://api.manus.ai/v2/task.create`, then polling
`GET /v2/task.listMessages` or receiving webhooks. Status can be running, stopped,
waiting for user input, or error. This is not a synchronous SERP contract.

A live unauthenticated creation request returned:

```http
POST https://api.manus.ai/v2/task.create
Content-Type: application/json

{"message":{"content":"Rust E0382 use of moved value"}}

HTTP 401
{"error":{"code":"unauthenticated",
 "message":"missing authentication: require either API Key or Bearer Token"},
 "ok":false}
```

The request ID was removed from the fixture. All 24 matched API probes returned
this authentication denial. They validate the documented API access boundary;
they are **not** a benchmark of a nonexistent public Manus SERP adapter. There is
no captured successful Manus source schema, order, snippet coverage, or task
runtime. The credentialed asynchronous API is a separate possible product scope,
with account access and billing/quota dependencies; paid access was not tested.

## Query semantics and filters

| Property | Felo | Manus |
|---|---|---|
| Complete native input | English browser query preserved; six-case JSON roundtrip tests pass | Six-case JSON roundtrip tests pass; anonymous UI submits then redirects |
| Multiword / quotes / punctuation / Unicode / `site:` | All transported unchanged by the prototype; direct live requests blocked | All transported unchanged in API message; live requests denied |
| Search-language semantics | Not established; observed query rewriting can alter semantics | Not established; agent prompt interpretation is not native operator support |
| Region | `langcode` observed, but not evidence of geographic filtering; unsupported | Unsupported/unverified |
| Recency day/week/month/year | No verified mapping; unsupported | No verified mapping; unsupported |

The cases include `"use of moved value" Rust`, `C++ std::vector café 日本語`, and
`site:doc.rust-lang.org E0382`. Transport tests cannot prove upstream phrase,
Unicode-tokenization or site-filter fidelity. Both adapters would have to reject
region/recency flags until verified. Local positive `site:hostname` enforcement
would still be required by Kestrel's contract.

## Repeated matched measurements

Environment: local macOS arm64, Rust 1.96.0, base commit `95899d1` plus this
investigation. Network egress geography was not verified. No credentials used.

- Six versioned queries: general science, coding, fresh Python information,
  quoted phrase, punctuation/Unicode, and `site:`. The fresh query is copied from
  the existing benchmark corpus. Inputs are shared across all four providers.
- Window 1: **13:00:22–13:00:55 UTC**, September 12. Window 2:
  **13:02:52–13:03:13 UTC**, after the observed cooldown. These are two short
  same-day windows, not multi-day availability evidence.
- Each window schedules six queries with fresh clients and six with reused
  clients per provider: 24 scheduled calls/provider, 96 total. Provider order is
  reversed in window 2. Concurrency is one, pacing 300 ms between completed
  calls, budget 5 seconds, no page fetch/rank. Fresh/reused means HTTP client and
  connection-pool lifetime, not clean/reused browser cookie jars.
- Felo/Manus probes use reqwest/rustls, fixed Chrome 148 Mac user agent, no cookie
  jar, no redirects, no retries, bounded 2 MB streamed bodies. Baselines use
  Kestrel's normal randomized coherent headers and pooled transport. This is
  a functional matched-query comparison, not a controlled TLS-profile experiment.
- **Measurement defect disclosed:** window 1 recorded `Retry-After` but failed to
  honor it, generating 11 HTTP 429 observations. The corrected runner suppresses
  later calls until the cooldown expires; window 2 has one 400, one 429 and ten
  skipped calls. The first window's block rate must not be interpreted as a
  natural provider failure rate. Corrected code is the reproducible runner.

| Provider / surface | Scheduled | Attempted | Nonempty | Other outcomes | Successful p50 / p95 (s) | Other p50 / p95 (s) |
|---|---:|---:|---:|---|---|---|
| Felo public frontend | 24 | 14 | 0 | 2 token-required HTTP 400; 12 HTTP 429; 10 cooldown skips | N/A | 0.434 / 0.715 |
| Manus API without key | 24 | 24 | 0 | 24 HTTP 401 | N/A | 0.335 / 0.390 |
| Swisscows | 24 | 24 | 24 | none in provider diagnostics | 0.200 / 1.009 | N/A |
| Bing | 24 | 24 | 20 | 4 locally site-filtered empties | 0.313 / 0.447 | 0.245 / 0.446 |

Felo/Manus denominators are exact HTTP requests (38 combined, zero redirects or
retries), excluding exploratory requests, entry probes, asset downloads and page
reads. Baseline counts are logical provider searches; raw HTTP status/redirect
counts were not captured. All baseline diagnostics report zero retries. Bing's
four empties each discarded ten off-domain rows: **not valid upstream empties**.
No timeout, transport error, body-limit, malformed response or valid upstream
empty was observed in the paired run. Felo's token challenges are 2/14 attempts,
rate limits 12/14, usable-result coverage 0/24 scheduled; Manus denies 24/24.
These counts overlap neither with skipped requests nor successful retrievals.
Percentiles use median and nearest-rank p95 and are unstable at these sample sizes.
Fast denials are not fast searches.

Fresh/reused all-outcome p50 seconds respectively: Felo 0.532/0.288 (8/6 actual
attempts, strongly confounded by rate limiting), Manus 0.343/0.121 (12/12),
Swisscows 0.536/0.110 (12/12), Bing 0.355/0.263 (12/12). No conclusion about browser
session reuse is supported. An isolated fresh-browser quality matrix is blocked
by security/guest quotas and is unnecessary to reject the current HTTP-only scope.

### Quality at top five

44 distinct query/URL pairs were manually inspected using titles, URLs and
snippets; explicit labels are versioned in `judgments.json`. The mixed Unicode
stress query has no single clear relevance target and is excluded from quality
scores (four calls/provider), while remaining in access/latency measurements.
No keyword-overlap score substitutes for judgment. Relevance is not full factual
accuracy or page-body verification; for example, one sky-colour snippet has
incorrect wavelength wording despite a relevant topic.

| Provider | Usable coverage, judged scheduled calls | Mean P@5, scheduled | Mean P@5, conditional on nonempty |
|---|---|---|---|
| Felo direct | 0/20 | 0 | N/A |
| Manus unauthenticated API | 0/20 | 0 | N/A |
| Swisscows | 20/20 | 0.92 | 0.92 |
| Bing | 4/20 | 0.20 | 0.25 |

A usable call contains at least one judged relevant top-five result; absent slots
count as zero. Swisscows returned relevant technical and official Python pages;
its historical Python 3.14.0 release and generic error-code index were marked
nonresponsive to the precise requests. Bing's coding, phrase and fresh queries
returned game/general-Rust, dictionary and general-news results respectively.
No provider-default change follows from this small set. Felo browser extraction
quality (discussed above) is excluded from this HTTP comparison.

## Contract fit, maintenance and decision

| Concern | Felo | Manus |
|---|---|---|
| Keyless HTTP fit | Fails at anonymous browser-token gate and guest quota | Fails at login/API authentication gate |
| Source contract | Completed-page sources validated; successful creation SSE not captured | No successful source-result contract validated |
| Budget / cancellation | Probe body and total timeout bounded; browser bootstrap outside 5-second budget; dropping a stream may not stop remote generation | Task creation/polling has remote lifetime; cancelling local HTTP does not establish task cancellation |
| Retries / fanout | Do not retry token-required responses; honor 429; repeated guest calls can exhaust quota | Do not retry 401; task retry can duplicate work unless supported idempotency is designed |
| Provenance / dedup | Preserve input query and original rank; canonical URL dedup in Kestrel; never use answer citations as fabricated snippets | Requires a validated source schema before provenance can be assigned |
| Maintenance | High: undocumented proxy, security bootstrap, guest quota, SSE recovery, Next.js props | High for scraping; separate credentialed agent integration and lifecycle management |

**Felo: no-go for the present keyless HTTP engine roster.** There is a working
source-extraction proof of concept for already-generated pages, but no standalone
HTTP path for new anonymous queries. Browser-backed search would require a new
runtime contract and still faces a guest quota. Reconsider only after a supported
keyless endpoint or an explicitly authorized credentialed/browser product scope.

**Manus: no-go for the present roster.** Actual anonymous prompt submission and
repeated API requests both establish authentication barriers; the documented
alternative is asynchronous agent work. It should not share a Felo adapter.

Because neither is viable within scope, production Engine/CLI/library registration
is intentionally absent. If scope changes, Felo's documented key API adapter is
roughly 1–2 engineering days plus independent quality/budget validation; a
browser-backed route is several days of infrastructure work with unresolved quota
risk. Manus's credentialed task integration is roughly 3–5 engineering days plus
validation (polling, cancellation, idempotency, source schema, credentials and
billing controls). These are estimates, not approval to build either alternative.

## Reproduction, fixtures and validation

```sh
cargo test --example provider_entry_probe --example provider_feasibility
python3 -m unittest discover -s benchmarks/provider-feasibility -p 'test_*.py'
cargo clippy --example provider_entry_probe --example provider_feasibility -- -D warnings
cargo run --example provider_feasibility -- window-1 > /tmp/felo-manus-window-1.jsonl
# Run window-2 later, after all Retry-After deadlines have elapsed.
cargo run --example provider_feasibility -- window-2 > /tmp/felo-manus-window-2.jsonl
python3 benchmarks/provider-feasibility/felo_extract.py tests/fixtures/providers/felo-thread-sanitized.html
python3 benchmarks/provider-feasibility/summarize.py benchmarks/provider-feasibility/observations.jsonl
```

The checked-in observations are sanitized, curated top-five projections with
original total result counts. Full captures remain ignored under
`benchmarks/investigation-40/`. The corrected runner skips rate-limited calls;
repeating it should not reproduce the original pacing defect. New query/URL pairs
need explicit judgments before quality summarization; the script fails on missing
labels instead of inferring relevance.

Live fixtures: Felo token-required 400, Felo API-key denial 401, Manus API denial
401, and sanitized Felo completed-thread HTML. Tests cover source order, exact
query transport, missing title/snippet without invention, invalid URLs, unknown
HTML, malformed JSON, incomplete threads, no-source ambiguity, challenges and
HTTP errors. Non-live negative cases are test mutations/synthetic inputs. A live
valid-empty or malformed provider response was not observed and is not invented.

Acceptance resolution: reproducible findings, concrete blockers, actual response
fixtures, minimum extraction proof of concept, measurements and separate no-go
recommendations are delivered. Full successful-query/operator/multi-day quality
validation and production implementation are **inapplicable under the demonstrated
access blockers**, not silently claimed complete. This report closes the
feasibility decision; any credentialed/browser expansion requires separate scope.
