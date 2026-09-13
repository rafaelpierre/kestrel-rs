# Interrupted-search recovery contract (#121)

This is a design for #122, #123 and #125, **not a shipped recovery feature**.
The baseline audited here is `e7527719feff0265ea34ee7334f289f076162b76`.
Existing public APIs, ordinary JSON, defaults and generated CLI help are unchanged.
The parent is [#70](https://github.com/rafaelpierre/kestrel-rs/issues/70).

## Current flag and persistence matrix

| Invocation | Provider persistence | Page persistence | Restart behavior |
| --- | --- | --- | --- |
| `search QUERY` | None | Disabled | Repeat provider and page work |
| `search QUERY --no-fetch` | None | Disabled | Repeat providers; no page work |
| `search QUERY --cache-ttl 300` | None | Eligible pages written after the fetch batch returns | Repeat providers; reuse fresh previously written pages |
| Above plus `--cache-dir PATH --cache-max-entries 1000` | None | Same, in selected directory/capacity | Same |
| `--cache-dir` or `--cache-max-entries` without TTL | No requests | Usage error, status 2 | Add TTL |
| `--no-fetch` plus any explicit page-cache option | No requests | Usage error, status 2 | Remove page-cache options or enable fetching |
| `fetch URL` | Not applicable | No cache flags or persistence | Repeat fetch |

`PageCache` stores text keyed by search-canonical URL and content limit. Its TTL
uses file modification time. Reads ignore missing/unreadable/expired entries;
writes and prune errors are currently ignored by the cached client. Pruning is
post-batch and not a strict instantaneous disk bound. Byte-capped pages are
excluded. A successful cache rename is not currently followed by file/directory
sync, so this implementation does not claim power-loss durability.

The fetch budget starts after cache reads and excludes subsequent writes and
prune (#18). Search URL canonicalization removes trailing slashes and tracking
parameters; using that identity for page text can alias distinct resources (#19).
Neither defect is corrected by this contract-only slice.

## Baseline reproduction

Run from this worktree:

```sh
cargo test --lib recovery_audit::interrupted_work_is_repeated -- --nocapture
```

The test starts a loopback HTTP server and fresh unit-test subprocesses. For
pages, a fast response is extracted while a second response remains blocked.
A test-only observer records the actual extracted text before the batch can
return. For providers, a Bing response publishes a normalized complete card into
the real streaming collector while its HTTP body remains open. The observer runs
after the collector installs the snapshot, not when the server sends bytes.
The parent waits for the observer's atomic event file, abruptly kills and reaps
the child, and restarts with the same storage directory and endpoint.

Assertions require identical extracted/accepted evidence on both invocations,
no persisted cache directory, and exact aggregate request counts: fast page **2**,
slow page **2**, provider **2**. A page commit would permit fast-page count **1**;
a partial provider stream must still be requested again to complete it, but its
previously accepted records should be available *before* that retry returns.
The baseline has neither that replay nor a durable provider store.

The provider helper exercises the actual adapter and collector through a local
endpoint; it is **not the public CLI** and does not add endpoint override flags.
The page helper exercises the public cached client API used by the CLI. These
are synchronized subprocess reproductions of the loss boundaries, not full CLI
recovery acceptance. No sleeps decide when extraction or acceptance happened;
short polling only waits for an explicit event, with a 15-second failure bound.
Observers and helpers are compiled only under `cfg(test)`.

This baseline test intentionally asserts today's loss. Replace its loss
assertions with committed-replay assertions as the dependent slices land. #125
must additionally exercise the CLI, internal deadline, dropped caller future,
graceful termination, repeated interruption, and committed-state synchronization.
SIGKILL alone is not evidence for those other termination paths.

## Proposed enabling contract

Add separate search-only opt-in flags in #123/#125:

- `--recovery-ttl SECS`: enable provider progress storage, positive finite TTL.
- `--recovery-dir PATH`: requires recovery TTL; default `~/.cache/kestrel/search-v1`.
- `--recovery-max-entries N`: requires recovery TTL; default 1,000 provider units.

They work with `--no-fetch`; existing page-cache flags retain their conflicts.
Without recovery TTL, do not read, create, mutate or prune provider storage.
Page persistence remains independently enabled by `--cache-ttl`. Direct fetch
and byte-prefix resumption remain out of scope. Flag names are proposed here;
implementation must update help, generated skill and parser/installation tests
in the same PR. Do not teach these recipes as executable until then.

A proposed initial/retry pair repeats the same command and directory:

```text
kestrel search 'rust ownership' --no-fetch --recovery-ttl 300 --recovery-dir ./progress --search-budget 3
kestrel search 'rust ownership' --no-fetch --recovery-ttl 300 --recovery-dir ./progress --search-budget 3
```

A page-enabled retry additionally repeats `--cache-ttl 300 --cache-dir ./pages`.
Every invocation receives a fresh finite budget; stored state never supplies an
old absolute deadline. Recovery is a performance/reliability aid, not a guarantee
that every provider will finish or that extracted text answers the query.

## Versioned identities and compatibility

Persist typed, bounded envelopes, not serialized internal futures. Validate both
the digest and the full embedded identity before accepting records.

| Unit | Required identity | Controls deliberately applied at replay |
| --- | --- | --- |
| Provider | Storage schema version; adapter/normalization version; exact normalized query sent to provider; provider name; effective region and time filter; request endpoint/configuration fingerprint; coverage definition | Current query/provider order, minima, deadlines, concurrency and final ranking |
| Page | Storage/extraction version; conservative request URL; content character limit; response byte allowance; extraction-affecting transport/representation settings | Current candidate order, ranking and fetch budget |

Use a structured serialization with unambiguous field boundaries and SHA-256
filenames. Store individual query/provider units so reordered or extended query
lists can reuse compatible units while current ordering determines output.
Do not persist historical query/provider *indices* as current ordering authority.
Preserve original provider ranks and source occurrences, including duplicates
across queries/providers. Query normalization must be exactly the current search
normalization; no extra case folding, keyword rewriting or whitespace changes.
Relative time filters require a stored request timestamp and an explicit coverage
anchor: conservatively miss when the effective time window changes.

#19 owns page identity: parse with `url::Url`, remove only fragments, preserve
trailing slashes, parameter order and duplicates, tracking parameters and encoded
path distinctions. Use only URL-library normalization justified by URL semantics;
never use search deduplication as a content identity. Do not infer redirect
aliases. Initially require exact page limits even if a larger cached extraction
could theoretically satisfy a smaller one. A byte-capped response is ineligible
under all allowances. An extraction-version bump invalidates old text; no fallback
to legacy canonical keys. Search result deduplication itself remains unchanged.

#121 defines these interfaces; #18 owns absolute cached-operation deadlines and
bounded cache I/O, #19 owns conservative page keys, #122 owns incremental page
commits, #123 owns provider commits, and #125 owns replay and retry orchestration.
No new dependency is required by this design document. Implementation must justify
any storage dependency against Rust 1.89 and existing atomic-file patterns.

## State and coverage transitions

Each provider attempt has a unique generation, increasing snapshot sequence,
record set, request/commit timestamps, and state `incomplete`, `complete` or
`invalid`. Empty complete results are valid completed work. Failure/challenge or
malformed EOF retracts that attempt's snapshot; commit an invalid tombstone before
claiming the failed records remain excluded across restart.

| Event | Durable transition | Replay/retry behavior |
| --- | --- | --- |
| Accepted snapshot queued | None yet | Do not call it committed |
| Snapshot atomically committed | Incomplete generation + complete normalized records | Seed collector, retry unfinished unit if target still unmet |
| Later snapshot/retraction | Replace same generation, higher sequence | Never union obsolete snapshots |
| Successful EOF | Atomically commit final snapshot and complete marker together | Skip request within compatible coverage |
| Failure/malformed EOF | Commit invalid generation, no reusable records | Retry with new attempt budget |
| Deadline/caller cancellation | Last committed incomplete snapshot survives | Count valid replay toward current minimum |
| Crash before commit | Previous whole committed generation or miss | Never accept partial files |
| Expiry/eviction/corruption/version mismatch | Miss | Retry; report reason |

A crash between observing a provider failure and durably writing its tombstone
can leave the prior incomplete snapshot. That is an explicit commit/loss boundary;
no guarantee can make an uncommitted retraction survive a crash. Recovery must
identify such evidence as incomplete, never completed provider work.

A complete marker means completion of the actual request's documented coverage,
not exhaustion of the provider's global index. If adapters use requested result
limits/pagination, include those in coverage identity; larger requests miss unless
coverage is proven sufficient. Today's early stopping before EOF is always
incomplete. Larger minima replay committed partial records and retry unfinished
units; smaller minima can stop from replay alone. Complete compatible units need
not be re-requested merely because other units still need more candidates.

Concurrent processes never merge incompatible generations. Serialize read/modify/
commit for a unit with an OS-managed cross-process lock released on process death,
then reject stale sequence updates. A generation that started earlier cannot
overwrite a newer generation's complete state. Avoid lock files whose mere presence
is considered ownership after a crash. Replay sees an immutable validated snapshot;
provider snapshot replacement remains the collector's authority for deduplication.

## Bounded storage lifecycle

Initial proposed limits: one writer per store per invocation, a queue of 16 owned
snapshots, one in-flight commit, maximum 4 MiB serialized provider envelope, and
1,000 entries by default. A producer pauses at the queue limit; no task per result.
Larger envelopes are skipped with a bounded diagnostic and remain nonrecoverable.
This bounds queued serialized storage payloads to 68 MiB plus collector state.
Page payloads remain bounded by extraction limits; implementations must also bound
their aggregate queue bytes, initially 16 MiB plus one capped in-flight page.
An over-limit page remains a returned result but is not cached. These are initial
implementation targets, not current executable defaults.

Start writing the first accepted snapshot immediately. Coalescing may replace
queued snapshots of the same generation with a newer one, but must preserve
retraction/EOF order. Completion and its records occupy one atomic envelope.
Use blocking workers for serialization/locking/fsync/prune and bounded admission;
async providers can progress while one write waits, until backpressure applies.
Do not hold collector mutexes while waiting on storage.

Commit uses a unique temporary file in the destination directory, write + file
sync, atomic rename and directory sync where supported. Only acknowledge commitment
after the documented durability steps succeed. Document platform/filesystem limits;
rename alone supplies reader atomicity, not universal crash/power-loss durability.
Malformed/oversized/truncated envelopes are misses, never partially parsed records.
TTL is checked against committed timestamps (future timestamps miss). Reads do
not refresh TTL. Prune under the store lock, exclude in-progress temporaries, and
clean orphan temporary files under a bounded maintenance policy. Eviction is an
expected recovery miss, not corruption. Entry and total-byte limits require
admission/eviction before acknowledging a new commit, with bounded scan work.

#18's single absolute deadline begins at operation entry, including cache reads,
network, enqueue, drain and foreground maintenance. Recovery read/commit time is
charged to the search deadline; page storage is charged to the fetch deadline.
A missing total deadline still uses a finite storage timeout (initially 250 ms
per lock/commit and 250 ms total normal-exit drain). Normal early stopping drains
within the smaller of remaining foreground budget and that drain bound. Deadline
expiry/caller cancellation closes admission and does not wait for a fresh drain
allowance. Already-running blocking syscalls may finish after cancellation; bound
their count, never promise their completion, and retain only acknowledged commits
in diagnostics. SIGKILL cannot drain. Graceful CLI shutdown must explicitly run
bounded cancellation/drain; runtime teardown is not a persistence guarantee.

## Diagnostics and dependent acceptance

Keep ordinary JSON unchanged. Use bounded stderr summaries for recovery hits,
incomplete/complete units, skipped requests, enqueued/committed counts and misses
(`disabled`, `absent`, `expired`, `evicted-or-absent`, `incompatible`, `corrupt`,
`oversized`, `storage-timeout`, `storage-error`). Absence cannot prove eviction.
Avoid echoing private queries/page text or credentials. Corruption/expiry retries
are automatic; storage errors advise checking directory access, and incompatible
entries advise a fresh run with consistent flags. Do not recommend deleting a
shared directory as routine recovery. Diagnostics describe this invocation's
observations; another process can commit after its deadline.

Before #125 is ready, synchronized subprocess tests must verify: completed warm
retry has zero requests; partial replay plus retry has exact request counts;
metadata-only and multi-query ordering/provenance; changed filters/versions/limits;
repeated interruption; failures and snapshot retraction; byte-cap exclusion;
expiry/eviction/corruption; simultaneous writers; and disk failures. Exercise
internal deadline, caller cancellation, graceful shutdown and abrupt termination
separately. Install the generated skill into a temporary project and execute
initial/retry recipes using the final CLI and local fixtures. The ten-question
live evidence gate supplements those deterministic tests; it does not establish
crash recovery correctness.
