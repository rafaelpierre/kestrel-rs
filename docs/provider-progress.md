# Recovering interrupted searches (#123 and #125)

Enable provider recovery independently of page caching, including metadata-only
searches. Repeat the same command after interruption:

```sh
# Initial invocation
kestrel search "rust ownership" -q "rust borrowing" --no-fetch --recovery-ttl 300 --recovery-dir ./progress --search-budget 3
# Retry with a fresh three-second budget
kestrel search "rust ownership" -q "rust borrowing" --no-fetch --recovery-ttl 300 --recovery-dir ./progress --search-budget 3
```

For page evidence, enable both stores and repeat both directories and allowances:

```sh
kestrel search "rust ownership" --recovery-ttl 300 --recovery-dir ./progress --cache-ttl 300 --cache-dir ./pages --content-limit 3000 --max-response-bytes 4000000 --search-budget 3 --fetch-budget 5
# Retry
kestrel search "rust ownership" --recovery-ttl 300 --recovery-dir ./progress --cache-ttl 300 --cache-dir ./pages --content-limit 3000 --max-response-bytes 4000000 --search-budget 3 --fetch-budget 5
```

`--recovery-ttl` must be positive and finite. Directory and capacity flags require
it; defaults are `~/.cache/kestrel/search-v1` and 1000 provider units. Without it,
provider storage is untouched and discovery repeats normally. Page cache flags
remain independent and conflict with `--no-fetch`. Direct single-provider library
`search` and standalone CLI `fetch` do not use provider recovery. The library
uses `KestrelClient::with_recovery(SearchRecovery)` for multi-query calls.

## Replay and necessary requests

Validated committed records enter the existing collector in the current query
and provider order. They retain original ranks/source occurrences and count toward
the current per-query minimum. Reordered or extended query lists reuse compatible
units; cross-query/provider duplicates merge through the ordinary collector.

Compatible complete units skip their requests, including completed empty responses.
Incomplete units retry only if the current minimum is still unmet. A smaller target
can return from incomplete replay without any request. A larger target retries
unfinished units with a fresh invocation deadline. Completed first-response coverage
is not proof that the provider's whole index is exhausted; increasing a target does
not introduce pagination or re-request an already complete compatible response.
A new stream snapshot replaces its unit's recovered partial state. Failed/malformed
outcomes retract that unit; deadline cancellation retains its valid partial records.

Keys include schema/adapter version, exact normalized query, provider, region,
time filter and built-in endpoint/coverage version. Changed settings safely miss.
Relative time filters (`d`, `w`, `m`, `y`) always miss because their moving window
has no stable adapter coverage anchor. Page keys additionally require exact
character and response-byte allowances; byte-capped pages are never cached.
No byte-range or compressed-prefix resumption is implemented.

## Commit and cancellation boundaries

Accepted means current collector state; enqueued means bounded storage admission.
Only an acknowledged atomic commit establishes retained storage. Streaming records
are queued before provider EOF; batch results are queued when available. Complete
records and completion metadata occupy one checksummed envelope. Errors write an
empty invalid tombstone; snapshots replace rather than union obsolete records.
A crash before a retraction commits can leave the prior incomplete snapshot.

OS locks serialize generation/sequence comparison, replacement and eviction. Older
generations cannot replace newer state. Temporary-file sync, atomic rename and
Unix directory sync provide the documented commit boundary; filesystem/hardware
failures and power-loss behavior on other platforms remain outside that promise.
Checksums cover metadata and records. Expired, future-dated, invalidated, corrupt,
oversized, incompatible and evicted/absent entries miss, with stderr reasons.

One joined provider writer admits 16 snapshots and 64 MiB, with a 4 MiB envelope
cap and at most one additional in-flight copy. The page writer admits 16 entries
and 16 MiB. Blocking jobs retain four admission permits per store after async
cancellation. Storage and queue waits consume the same absolute search/fetch
budget as their operation. Without a total budget, each write/maintenance wait
and final drain is at most 250 ms. Maintenance scans at most 4096 entries;
capacity remains best effort. Reads do not refresh TTL, which starts at snapshot
queue time. No detached async writer is left running.

With provider recovery enabled, Ctrl-C and Unix SIGTERM request cancellation,
stop provider/page collection and allow at most 250 ms of graceful drain before
status 130. Already-running filesystem calls can finish later; runtime shutdown
may wait for them. No late commit is guaranteed or counted as acknowledged.
Dropping a library future immediately drops its queued work; `SearchRecovery::cancel`
requests graceful cancellation instead. Construct a new store/client for a fresh
invocation after explicit cancellation. SIGKILL cannot drain.

Ordinary JSON shapes are unchanged. Provider diagnostics describe requests made in
this invocation, so a fully recovered run has no new provider request rows. Recovery
counts, skipped requests, miss reasons and commit failures appear on stderr. A hit
means compatible retained text, not proof that it answers the search question.

## Deterministic validation

`cargo test --all-features --test recovery_cli` launches the actual CLI against
loopback fixtures, checks committed files before killing processes, and verifies
exact request counts, results, ranks/provenance and completion. It covers repeated
interruption, smaller/larger targets, partial page recovery, corruption, expiry,
eviction, changed settings, concurrent processes, internal deadlines and graceful
signals. Library tests separately cover dropped callers and bounded storage.

The opt-in `test-fixtures` Cargo feature enables a loopback-HTTP-only endpoint
injection for these subprocess tests; ordinary release builds do not include it.
Fixture endpoints enter the recovery identity, preventing fixture/live cache reuse.
Tokio's signal feature adds signal-hook-registry (declared MSRV 1.26) to support
bounded graceful CLI shutdown; project MSRV remains 1.89.
