# Bounded empty-discovery policy investigation (#79)

This is a benchmark, not a production retry implementation. The JSON-lines
`discovery_probe` example retains a production `KestrelClient`; Python schedules
calls and reads existing lifecycle files after a bounded diagnostic flush.
Production CLI flags, search behavior, generated skill and JSON remain unchanged.
Use [#189](https://github.com/rafaelpierre/kestrel-rs/issues/189) for rollout.

```sh
cargo build --release --locked --features test-fixtures --example discovery_probe
python3 benchmarks/empty_discovery.py --binary target/release/examples/discovery_probe \
  --output /tmp/empty-discovery-fixtures --rounds 1
python3 benchmarks/empty_discovery.py --binary target/release/examples/discovery_probe \
  --output /tmp/empty-discovery-live --live --rounds 1
python3 -m unittest discover -s benchmarks -p test_empty_discovery.py
```

Use fresh directories. The feature enables only loopback provider injection;
live runs remove that environment variable. Each query/arm starts a fresh
process. Within `retry_reuse`, the second call keeps that process and client.
`retry_fresh` repeats the same probe in a new process; it measures client/process
reconstruction, not production CLI argument parsing or an actual model turn.
Both start all transport pools, including Yahoo, as default live discovery does.

Before execution, `POLICY` fixes these conditions:

| Arm | Discovery policy |
| --- | --- |
| short | One 1-second provider budget |
| long | One 5-second provider budget; in-flight requests remain active past 1 second |
| retry_fresh | 1 second, then 250 ms backoff and at most 3.75 seconds in a new process |
| retry_reuse | Same stages, with the original process/client |

All arms share a 6-second external work allowance starting before initialization,
plus at most 100 ms process cleanup. The next budget shrinks to the remaining
allowance, reserving 200 ms for diagnostic flushing. There are at most two calls,
at most three existing application sends per provider per call, so the common
ceiling is six sends per original provider/query (54 for nine engines). Single
attempts naturally spend less of that common ceiling. Redirects/internal HTTP
retries are not application sends; request counts do not bound redirect hops.
The harness does not change the existing inner retry policy.

Collection minimum is one unique valid candidate; no body fetching, ranking,
page caching or persisted recovery is enabled. Keep every returned candidate
with its provenance. This compares discovery, not answer completeness. An
accepted URL is not judged relevance. Retain unknown relevance until manually
inspecting titles, URLs and snippets. The canonical evidence gate separately
uses its own frozen minimum 20 and page-reading policy.

Only deadline outcomes for an empty query can trigger a second call. Provider
outcomes with observed authentication blocks, rate limits, Retry-After or detected
challenges are excluded even if their backoff later reaches the deadline. A
complete zero-result response, hard failure, unknown outcome, caller cancellation,
partial discovery or empty final body ranking does not independently trigger
recovery. Missing traces/failed flushing prevent retries. Exact query bytes and
engine membership are preserved; retry engine ordering is stable and sorted.
There is no syntax fallback or query rewriting. Tests cover multi-query eligibility;
the measured matrix intentionally uses one query at a time.

Seven local HTTP/1.1 fixtures expose actual requests and connection identities:
early result, 1.4-second result, first request stalled for seven seconds then
success, every request stalled, explicit empty result, HTTP 403, and HTTP 429
with Retry-After 30. The transient fixture is intentionally favorable to retry;
it is not a model of live failure frequency. The long arm directly exercises
preserving work past the short checkpoint without restarting it. Terminating a
probe bounds caller work; fixture threads can still finish their already-started
sleep. Keep fixture and live results separate.

The live matrix uses the ten exact canonical FTS queries, all nine providers,
one sequential call at a time and 250 ms pacing between cells. Arm order rotates
by query and round; one round does not completely balance ten queries over four
positions. Different live arms need not receive identical candidates or upstream
state. Record p50/p95 with nearest-rank ceil(p*n), including empty/error/censored
runs. At small n, p95 is effectively a maximum, not a reliable population tail.
No causal live latency or performance improvement follows from one window.

Each record retains UTC start, monotonic process and discovery duration, exact
JSON requests, initialization time, all results/errors, lifecycle attempts and
raw stdout/stderr; server logs record loopback requests and connection IDs.
Trace send totals remain separate from server request/connection totals.
Connection IDs count local TCP connections, not measured TLS handshake duration.
Python orchestration and diagnostic overhead are included in wall time; actual
agent turn latency is unmeasured and must not be represented as zero.

Source review: the all-provider-failure path in `search_many_with_clients_in_run`
returns through `merge_outcomes(...)?` before exposing `SearchReport`. Production
recovery therefore needs a structured outcome API on failure; reading debug files
is solely an experimental workaround and unsuitable for production control.
