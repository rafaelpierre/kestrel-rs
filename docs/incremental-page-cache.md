# Incremental page commits (#122)

With search `--cache-ttl`, each eligible extraction is queued as it completes,
while other page requests continue. A single writer commits entries concurrently
with collection; there is no whole-fetch-batch write barrier. Standalone `fetch`
still has no persistent cache. Provider discovery still repeats in a fresh
process; provider records and completion recovery are separate #123/#125 work.

Repeat the same search with the same cache directory, character and response-byte
allowances to reuse unexpired committed page text:

```sh
kestrel search "rust async" --cache-ttl 300 --cache-dir .kestrel-cache --content-limit 3000 --max-response-bytes 4000000
# After interruption, run the identical command again.
kestrel search "rust async" --cache-ttl 300 --cache-dir .kestrel-cache --content-limit 3000 --max-response-bytes 4000000
```

An extraction is accepted when its text is retained in the current result; it is
enqueued when bounded writer admission succeeds. Neither event promises recovery.
A commit requires a complete checksummed JSON record, temporary-file flush/sync,
atomic replacement, and directory sync on Unix. The foreground summary counts
only acknowledged commits. The filesystem and hardware must honor these calls;
this is not a guarantee against arbitrary storage failure. On platforms without
directory sync, rename durability across power loss is weaker.

The versioned identity includes conservative URL, extractor version, exact
character limit and exact response-byte allowance. Byte-capped responses are
excluded. Character-truncated text can satisfy only the same allowance, never a
larger one. Records validate identity, content checksum, creation time and TTL.
Malformed, truncated, oversized, expired, future-dated or incompatible records
are misses. Legacy unversioned and page-text-v2 entries are not migrated or
replayed. Temporary files left by a killed process are ignored.

The queue admits at most 16 entries and 16 MiB of URL/text, including the entry
being written; a worker may own one additional copy. Individual URLs over 8192
bytes or entries above the byte allowance are skipped with a diagnostic. Queue
admission applies backpressure and is charged to the absolute fetch deadline.
The collector retains extracted text before waiting for admission. Storage uses
four admission permits per cache and its clones; running blocking workers keep
their permits after async cancellation.

When an explicit fetch budget exists, writes and final drain use its remaining
absolute time. Without one, each storage wait and final drain is bounded to
250 ms. The writer is joined to the operation, never a detached async task.
Dropping the operation drops queued work; a syscall already running can finish
later, without a guaranteed or reported late commit. Runtime shutdown can wait
for such blocking calls. See [deadline limits](cache-deadlines.md).

Writers and eviction serialize through an OS file lock with a 250 ms acquisition
limit, automatically released on process exit. Readers see old or new complete
entries. Maintenance runs only after acknowledged writes, scans at most 4096
entries, and removes the oldest excess entries in that window. Capacity is best
effort in oversized directories. Read-only/damaged storage and lock/write errors
produce misses or stderr notices, preserving extracted text and ordinary JSON.
An unacknowledged timed-out write is reported as uncommitted even if its worker
subsequently finishes.

The deterministic subprocess regression uses the real CLI parser and page
attachment stage with supplied local candidates (no live provider dependency).
It observes a complete fast-page entry while a slow HTTP response is blocked,
kills that CLI test process, then starts two fresh processes in the same cache.
It verifies one fast request, two slow requests, one then two hits, and identical
completed text. This demonstrates process-kill recovery, not power-loss safety.
Unit tests separately cover deadlines, retained permits after cancellation,
damaged/expired records, incompatible limits, concurrent handles and lock
failure/release. Full provider-discovery/CLI replay belongs to #125.
