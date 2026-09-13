# Provider progress storage (#123)

Search can now record provider snapshots independently of page caching:

```sh
kestrel search "rust ownership" --no-fetch --recovery-ttl 300 --recovery-dir ./progress --search-budget 3
```

`--recovery-ttl` enables recording and must be positive and finite. The directory
and capacity flags require it; defaults are `~/.cache/kestrel/search-v1` and 1000
units. Without it, no provider storage is touched. Direct single-provider library
`search` and standalone CLI `fetch` do not use this store. The library enables
multi-query recording with `KestrelClient::with_recovery(SearchRecovery)`.

This slice records but does not replay provider work. #125 adds restart replay,
current-minimum accounting and compatible request skipping. Repeating this
command currently still invokes providers. Page recovery from #122 is separate.

The collector queues cumulative normalized snapshots before acknowledging the
provider's next read. Batch adapters enqueue their records immediately on return.
The writer is polled alongside all query collectors, so disk waits do not prevent
network progress until queue backpressure applies. No collector lock spans I/O.
Each query/provider unit retains its original ranks and source occurrences;
current result merging handles duplicates across units. Snapshot replacement is
not append-only union: removed records disappear in the next committed sequence.

Each checksummed envelope includes schema/adapter version, query, provider,
region, time filter, built-in request endpoint/coverage version, generation start
and UUID, increasing sequence, commit-queue timestamp, records and state.
Successful EOF commits final records and completion in one atomic envelope;
errors commit an empty invalid tombstone. Deadlines preserve the last incomplete
snapshot. Empty successful provider results are valid complete state. A later
invocation's generation supersedes an older one; stale sequence/generation writes
are rejected while holding the same OS lock as replacement and eviction.

The shared page/storage primitive uses bounded blocking-worker admission,
OS-managed locks (250 ms acquisition limit), same-directory temporary files,
file sync and atomic replacement, plus directory sync on Unix. It does not
promise power-loss durability on platforms without directory sync. Checksums
cover metadata and records; torn, corrupt, oversized and expired entries miss.
A crash before a retraction commit can leave the prior incomplete snapshot; it
must never be interpreted as complete provider work. Clock ordering is local
system-time ordering plus UUID tie-break, not a distributed consensus protocol.

One joined writer accepts 16 snapshots with a 64 MiB admission budget; each
snapshot is conservatively size-checked before copying and limited to 4 MiB on
serialization. One in-flight worker may retain another snapshot. Serialization,
checksums, locking and filesystem calls run on bounded blocking workers. Queue
backpressure respects the invocation's absolute search deadline. Without a
search budget, each storage wait and final drain is at most 250 ms. A running
syscall may complete after cancellation; only acknowledged commits are reported.
Maintenance scans at most 4096 entries; capacity is best effort. Failures and
oversized snapshots produce stderr diagnostics without changing ordinary JSON.

Tests cover real streamed records committed before EOF and retained after caller
cancellation or an abruptly killed fresh subprocess, snapshot replacement/retraction, atomic completion metadata,
corruption, generation ordering, TTL, competing handles and queue saturation.
The page-process kill test also validates the shared atomic primitive.
#125 adds full provider subprocess replay and repeated-interruption coverage.
