# Provider diagnostic records

Set `KESTRELSEARCH_PROVIDER_TRACE_DIR` to opt into diagnostic files. Normal CLI
result JSON and public `SearchReport` fields retain their existing shapes.
Every scheduled provider/query operation produces one `outcome-*.json`, including
all-failed searches, deadline expiry, caller cancellation and quorum cancellation.
Fallback providers that are never selected are not scheduled operations.

The existing outcome fields are retained. A nested `lifecycle` object has
`schema_version: 1` and contains the logical record, attempt records and timings.
Raw response captures, including Yahoo non-2xx bodies, use the same opt-in path.
They contain query text, final URLs and response bodies; they are not anonymized.
No response cookies or authorization headers are copied into diagnostic metadata.
Only Retry-After is retained from response headers. Body capture requires a
successfully read body; interrupted bodies still retain status in the outcome file.

## Correlation and counting

- `run_id` is a unique ID per search API invocation, shared by its provider jobs.
  `benchmark_run_id` optionally retains `KESTRELSEARCH_BENCHMARK_RUN_ID` for joining
  these invocations to benchmark artifacts.
- `search_id` identifies one scheduled provider/query operation. Fanout records
  are registered before polling, including jobs that never acquire a semaphore.
- Each `attempts` entry has the same run/search IDs, a unique `attempt_id`, and a
  one-based `ordinal`. Raw response metadata includes matching `correlation` IDs.
- `send_attempts` counts application calls to send, including retries. It does
  **not** count redirect hops or any retries internal to the HTTP client:
  `count_unit` is `application_send`, and `redirect_hops_observed` is `false`.
  `http_status` is the final status visible to that send, after automatic redirects.

Count each logical `search_id` once, using `logical_outcome`. Count attempts by
`attempt_id`, not by raw files plus outcome records: those are two views of the
same attempt. Attempt records with `response`, `transport_error`, `body_error`,
`cancelled`, `response_too_large`, or defensive `unknown` outcomes reconcile to
`send_attempts`.
Retries are `max(send_attempts - 1, 0)`. A failed attempt followed by recovery
is one successful logical search. A completed 500 followed by cancelled backoff
is one completed HTTP attempt and one cancelled logical search.

## Status, errors and challenges

An attempt retains `http_status` and the verbatim `retry_after` header as soon as
headers arrive, even if reading the body later fails or is cancelled. Recording
Retry-After does not change the existing retry policy. Bodies remain subject to
the provider response-size limit, including HTTP error bodies. An oversized body
ends the attempt and logical search as `response_too_large` without retries or
raw capture; observed status is retained, challenge classification stays unknown,
and the interrupted body interval is censored.

`challenge` is independent of status: `detected` for known provider challenge
markers, `not_detected` when a complete nonempty body has no known markers, and
`unknown` when content is empty, unavailable or incomplete. `not_detected` does
not imply a valid results page. Thus a 403 can simultaneously be an HTTP error
and a detected challenge, while a 200 challenge can produce a logical failure.

`transport_error` uses typed backend evidence: timeout, connect, DNS (when
exposed by the backend), TLS, body, decode, request, or unknown. TLS classification
uses the actual error types of both backends, not message substrings. A backend
that does not expose a more specific cause retains its broader category. No raw
transport error messages are added to the structured attempt record.

## Timing and cancellation

Each interval has a phase, optional attempt ID, elapsed milliseconds, and a
`censored` flag. Phases are `not_started`, `queue`, `send`, `body`, `parse`,
`processing`, and `backoff`. `send` measures application send to final headers
(including client connection/redirect work); `body` measures reading/decoding.
Processing includes existing tracing and bookkeeping; it is kept out of network
and parsing samples. Backoff is not part of send latency.

A censored interval records elapsed time observed before interruption, a lower
bound rather than a completed latency sample. Deadline and caller/quorum drops
censor the active phase; client timeouts censor the affected send/body phase.
Earlier intervals and completed attempts stay complete. Cancellation in queue
or before polling creates no send; cancellation during backoff creates no extra
attempt. `cancellation_phase` identifies the interrupted logical phase, and
`logical_outcome` distinguishes `deadline`, `cancelled_quorum` and
`cancelled_caller` from completed results, empty results and errors.

Do not combine censored intervals with completed latency samples. Also separate
response, transport-failure and body-failure samples when reporting distributions.
The legacy `elapsed_ms` field remains total logical elapsed time.

## Persistence boundary

The guard finalizes the record once and releases diagnostics/recorder locks
before passing snapshots to `capture_provider_lifecycle`. This change defines
record semantics; it does not introduce a second persistence implementation.
Writes still use the existing synchronous, best-effort opt-in sink. Issue #17
owns nonblocking persistence, overflow handling and shutdown/flush behavior.
Process termination or persistence failures can still lose files.

Mocked tests exercise both HTTP backends, correlation, redirects, retry recovery
and exhaustion, typed errors, challenges, interrupted bodies, backoff, queueing,
never-polled jobs, deadlines and quorum cancellation. No live-provider availability
claim follows from those tests.
