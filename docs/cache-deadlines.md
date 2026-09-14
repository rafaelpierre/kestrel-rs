# Cached fetch deadlines and bounded work (#18)

`fetch_all_cached` and `fetch_all_cached_detailed` share one absolute deadline
created at cached-operation entry when a budget is supplied. The deadline covers
cache reads, the remaining network phase, page writes, and foreground maintenance.
The CLI's `--fetch-budget` (default two seconds) uses this path when `--cache-ttl`
is enabled. Library calls without a fetch budget retain bounded storage admission
but have no total deadline.
Provider search, ranking, command initialization/output and runtime shutdown are
separate; this is a cooperative stage deadline, not a hard real-time process cap.

Previously, an unbounded `join_all` read cache files before the network timer
started. Sequential writes and a complete prune followed the network timer.
Even a fully warm request could substantially exceed its nominal fetch budget.
The implementation now:

- Reads through four concurrent futures, preserving each hit at its input index.
- Carries the same absolute deadline into the fetcher instead of resetting a
  full network allowance after storage.
- Retains each completed page result before queueing its incremental write.
- Stops waiting for writes/maintenance at the same deadline and retains text.
- Uses indexed hit metadata, avoiding repeated scans of the missing URL list.
- Skips pruning for hit-only calls and when no write succeeded.

`budget_exhausted` describes the entire cached stage, so it can be true even when
all selected pages have content but a write or maintenance timed out. `cancelled`
counts unfinished page work, not cache writes. If time expires while reading
cache entries, completed hits survive; unresolved entries count as misses and
unfinished page work, and no replacement network requests start. Public schemas
and flag names are unchanged. Storage-write/maintenance errors and storage
deadlines produce stderr notices; they do not discard page results.

## Storage admission and cancellation

A cache instance and its clones share four semaphore permits. Each operation
acquires a permit before creating its blocking job and transfers ownership to
that job. A cancelled async caller cannot release the permit of a still-running
filesystem syscall. Retrying through the same cache therefore cannot queue an
unbounded number of replacement jobs behind slow storage. Independently created
cache objects have independent limits; this is not a process-global or
cross-process concurrency cap. Network/parsing concurrency controls are unchanged.

Filesystem metadata, reading, writing, rename and pruning run on blocking workers.
Reads reject JSON entries larger than six bytes per requested Unicode scalar plus
64 KiB metadata and enforce that bound while reading, including a one-byte overflow
probe. They also reject malformed JSON, checksums/identity mismatches and text
exceeding the character limit. Expired entries are misses;
reads no longer unlink them, avoiding removal of a concurrent replacement at that
boundary. Writes retain atomic replacement and clean their own temporary file on
failure. Incremental writes, file synchronization and cross-process locks are described in
[incremental page commits](incremental-page-cache.md).

Maintenance inspects at most 4,096 directory entries and retains only text-entry
metadata from that window. It removes the oldest excess entries in the observed
window. Capacity remains best effort if the directory exceeds that window or has
concurrent writers; no exhaustive directory scan is allowed to extend the
foreground budget. Successful writes can trigger further bounded passes on later
calls. No maintenance runs in the background as a detached async task.

Dropping a caller or reaching a deadline cannot interrupt an already-running OS
filesystem call. Up to four admitted workers can finish afterwards; their late
writes are not guaranteed or reported as committed before return. A Tokio runtime
may wait for blocking workers on shutdown. The CLI can therefore outlive the stage
deadline on a stuck filesystem; callers needing an absolute process bound must
apply one separately. This limitation is distinct from charging storage waits to
the operation and bounding outstanding work.

## Validation and related changes

Deterministic tests hold all storage worker permits using explicit start/release
channels, run repeated short-budget warm reads, and verify bounded return, retained
worker admission and eventual reuse after release. A second synchronized test
warms one page, waits for the cold page's actual HTTP request, blocks storage and
then releases the response: the write deadline must preserve both texts, report
one cache hit and zero cancelled page fetches, and leave the new page uncommitted.
A maintenance test verifies cancellable admission and partial progress on a
5,000-entry directory without an exhaustive pass. Ordinary network tests use
local fixtures; no live provider is needed.

#19 owns conservative/versioned URL identity; this change preserves the current
key contract. #122 removes the fetch-batch persistence barrier and builds on these
admission/deadline rules. Provider-record storage/replay remains #123/#125. No
byte-range resumption, new dependencies, or Rust MSRV changes are included.
