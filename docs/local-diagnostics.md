# Local diagnostic persistence

Ordinary event logging and opt-in provider traces share one dedicated writer
thread per process. Request tasks and provider cancellation guards submit records
without waiting for filesystem I/O. Paths, event JSONL and provider trace schemas
are unchanged. Encoding still runs on the caller, with a bounded output size.

Default limits are 64 records being prepared or queued, 16 MiB of retained payload
bytes across preparation, queue and active writing, and 8 MiB per record (combined
raw body and metadata). The active writer may hold one additional record beyond
the queue limit, but its bytes remain charged. At most two files belong to one
record. Buffer allocation capacity can exceed payload length (standard Vec growth);
record count and individual buffer size remain bounded. This is not a bound on
caller-owned input objects or total process memory.

Admission reserves a queue slot before serialization. Full queues, byte-limit
violations and serialization failures drop the entire new record; they never wait
for disk or spawn a task per event. Earlier accepted records are retained. The
writer counts filesystem errors and continues with later records. A disk failure
may leave a partial file or a raw body without its metadata; writes are best effort,
not atomic transactions or fsync durability. Within one process, JSONL records are
serialized by the writer; no cross-process locking guarantee is added.

## Library policy and lifecycle

Configure once, before first logging or tracing use, preferably before starting
request tasks. Invalid limits, repeated configuration and worker startup failure
return an error. Default lazy startup is best effort; a startup failure disables
persistence for the process and is observable through stats/flush.

```rust,no_run
use kestrelsearch::diagnostic_sink::{self, Config};

# fn main() -> std::io::Result<()> {
diagnostic_sink::configure(Config {
    enabled: false,
    ..Config::default()
})?;
# Ok(())
# }
```

Set `enabled: true` and override the three limits to tune persistence. This is a
process-wide library policy, not a per-client setting. It does not change returned
search/fetch diagnostics, OpenTelemetry export or explicit benchmark artifacts.
There are no new CLI flags or environment variables.

After producers have stopped, call
`diagnostic_sink::flush(std::time::Duration::from_secs(1)).await` inside Tokio.
The timeout includes waiting for queue space and the writer's FIFO barrier. True
means all records ahead of the barrier finished their write attempts; inspect
`diagnostic_sink::stats()` for cumulative drops and write failures. Concurrent
producers can enqueue after the barrier, so callers must quiesce them to drain
all work. Cancelling or timing out a flush never cancels an admitted disk operation.

The CLI gives local diagnostics one second to flush after command handling,
before runtime shutdown. This is outside search/fetch budgets and JSON
`elapsed_seconds`. A flush timeout or recorded loss produces one stderr summary
and does not change the command's exit status or stdout. The process-lived writer
is detached: Drop never joins it, and process termination can lose pending records.
A hung filesystem can occupy that one thread; it cannot create replacement workers.

## Benchmarks and scope

Report request/encoding time separately from local diagnostic flush time and
whole-process duration when measuring tracing overhead. Moving writes to a worker
does not remove storage or serialization costs. A trace-enabled run is not an
untraced baseline. No live speedup is claimed by this change.

The explicit public `benchmarking::write_artifact` API retains synchronous return
and filesystem-error semantics. CLI search artifacts use that API after retrieval;
this change covers ordinary log events and provider trace capture, not that
separate artifact contract. OpenTelemetry has its own exporter/shutdown policy.
