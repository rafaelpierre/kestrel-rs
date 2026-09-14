# Bounded discovery retries (#189)

Empty discovery can be temporary when providers run out of budget. Kestrel now
retries only deadline-exhausted provider/query work, within one retained client.
It uses the structured attempt outcome produced by a typed SearchDeadline;
an arbitrary error string containing “deadline exceeded” is not eligible.

The trigger is per normalized query: no retained accepted candidate, and at
least one eligible deadline outcome. Eight deadlines plus one challenge retry
the eight deadline providers. Completed empty/filtered responses, challenges,
rate limits and other hard errors do not independently trigger recovery.
Any retained candidate stops recovery for that query, even below `--min-results`.
Other empty queries can recover concurrently without rerunning successful ones.
Original queries, constraints, ordering within attempts, canonical deduplication
and source provenance are preserved. Fetching and ranking never trigger retries.

| Setting | Attempt budgets (seconds) | Overall discovery allowance |
| --- | --- | --- |
| CLI default | 5, 10, 15 | 32 seconds |
| Explicit B < 15 | B, min(B+5,15), min(B+10,15) | Sum of budgets + 2 seconds |
| Explicit B >= 15 | B only | B |
| `--no-search-budget`; library default None | No shared deadline, one attempt | Individual request timeouts/retries remain |

There are at most three discovery attempts. Backoffs are 250–500 ms and
500–1,000 ms with integer jitter. Each attempt includes queueing, enabled
persisted-recovery I/O and existing request retries. All queries share the
invocation's overall absolute cap; a late retry gets only its remaining time.
At most three application sends per provider attempt means at most nine per
provider/query per invocation, excluding redirects and backend-internal retries.
The shared concurrency bound is unchanged. This is a cooperative async deadline,
not a process cutoff: initialization, final merge, output, bounded parser cleanup
and optional page fetching/ranking can add wall time.

Valid Retry-After delta-seconds/HTTP dates govern request backoff for both HTTP
backends. An attempt interrupted after 429 or Retry-After guidance reports
`rate_limited_deadline` and is excluded from discovery retries. This conservative
choice can leave recoverable work unfinished, but never resets a server's wait.
Detected non-2xx bot challenges stop request retries too. A server-advised delay over 15s ends that provider request without a retry,
rather than shortening the wait or stalling an unbudgeted caller. Invalid or
unrepresentable header values fall back to bounded request retry backoff.

Dropping the library future stops async work/backoff. Persisted-recovery
cancellation also interrupts backoff; existing CLI signal/drain behavior applies.
One persistence writer is shared across the invocation, preserving its 16-entry,
64 MiB queue bound. Query sequence numbers continue across attempts within that
generation. Replay and queue admission obey each attempt deadline; the shared
writer/drain obeys the overall discovery allowance. Persisted complete units
remain eligible for reuse under the existing recovery contract.

Stderr identifies query index, attempt, delay, next budget and eligible count.
Provider report/trace rows add `discovery_attempt`, distinct from HTTP `retries`.
All attempts and provider timings remain available. JSON query completion uses
latest provider observations, while provider outcome totals count all attempts.
Final all-failed errors group causes using latest provider/query counts, explicitly
labeled; a challenge from attempt one remains counted after other providers retry.
Runtime failures still exit 1 with no stdout/JSON error envelope. A completed
empty provider can still make an otherwise failed empty search exit successfully,
consistent with the prior empty-result contract.

Compatibility: short explicit budgets are now first-attempt budgets, so callers
must allow the stated extra time. Budgets >=15s preserve single-attempt behavior.
Library `ProviderSearchDiagnostic` struct literals must add `discovery_attempt`;
old serialized diagnostics deserialize it as 1. No result schema changes or new
CLI flags are introduced. Regenerate installed skills with the updated binary.

## Controlled cost comparison

The paused-clock `controlled_policy_cost_against_single_long_attempt` test compares
three simulated provider behaviors at the same 32s maximum allowance, ten trials
per behavior/policy, retaining empty/failure observations. No live-provider or
quality improvement is inferred. Its single candidate is fixed evidence; real
HTTP parsing, request retries, client initialization and disk I/O are outside
these simulated intervals. Jitter varies within the declared bounds. p50/p95 use
nearest rank (ceil(p*n)); with n=10 p95 is the maximum, not a reliable tail estimate.

One observed test run (milliseconds, request counts summed over ten trials):

| Provider behavior | Policy | Nonempty | Requests | p50 | p95 |
| --- | --- | --- | ---: | ---: | ---: |
| First request stalls, next succeeds | 5/10/15 recovery | 10/10 | 20 | 5,417 | 5,485 |
| First request stalls, next succeeds | Single 32s | 0/10 | 10 | 32,000 | 32,000 |
| Every request takes 8s | 5/10/15 recovery | 10/10 | 20 | 13,385 | 13,497 |
| Every request takes 8s | Single 32s | 10/10 | 10 | 8,000 | 8,000 |
| Every request stalls | 5/10/15 recovery | 0/10 | 30 | 31,161 | 31,259 |
| Every request stalls | Single 32s | 0/10 | 10 | 32,000 | 32,000 |

Recovery helps a transient stalled request but costs an extra request and about
5s for a consistently slow provider. It triples requests without improving
coverage for persistent stalls. This supports a strictly empty/deadline trigger
and finite caps, not a claim that retries are generally faster than a longer
attempt. The broader matched live policy/relevance study remains in #79.
