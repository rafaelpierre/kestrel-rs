# Short search-budget overhead: issue #80

## Finding

The approximately 140 ms excess is reproducible on this host and is principally
**fresh-process client initialization before the provider deadline**. It is not
evidence that a one-second provider budget permits another 140 ms of network
work. The controlled provider stage exceeded one second by a median 2.294 ms
without diagnostics, versus 133.349 ms median initialization. Enabling existing
diagnostics increased median provider-drop work from 0.005 to 0.233 ms per job.

Yahoo/primp construction dominates initialization. A separate control moved its
cached default-root-store loading before client construction, without changing
certificates or verification: subsequent first-client construction was 4.098 ms
median, while root loading itself was 155.622 ms median. Preloading relocates
work; it does not make a fresh process faster.

Keep the existing budget contract. Retain `KestrelClient` for related library
calls. [Follow-up #93](https://github.com/rafaelpierre/kestrel-rs/issues/93) scopes
avoiding eager Yahoo setup for CLI searches that exclude Yahoo. It does not
promise a speedup for the default all-provider search.

## Method and provenance

Measured September 13, 2026 on Apple M3 Max, macOS 26.6.2 arm64, Rust 1.96.0
(`ac68faa20`, LLVM 22.1.6), Python 3.14.7. Source base:
`f100f4cdeb96e2a660149c48e8a94de58e2eb7e8`. Builds use the locked dependencies,
release profile, thin LTO and stripping. The repository MSRV remains 1.89;
no manifest, dependency or shipping Rust source changed.

The [reproduction harness](../benchmarks/budget-overhead/README.md) creates an
isolated source copy. Only that copy contains endpoint substitutions and the
observer. The production binary does not understand the fixture environment
variable. The observer buffers timestamps and emits them after runtime shutdown.
It does not write per-event files or add public timing fields.

SHA-256 identifiers:

| Binary | SHA-256 |
| --- | --- |
| Unmodified CLI | `d7e0a383e7b36d4b1493a116e2afe0444a77e1d17c24765641336707b2232e09` |
| Instrumented CLI | `fb2b11a9fc7e9808fd10b8729f5c8ab4f8902ff6d9f2fc268210f447f94ccd2d` |
| Retained-client example | `6b92e0b36e2edbc31b88cb8470f03835feb552d15702a1b01297d28eb1f9e964` |
| Root-store control | `09fcf450c356f72456d8bb8b7980a88e4e0e62b7881bb80892ea9c90ca38d239` |

Controlled trials: 120 fresh CLI processes, plus 12 retained-client processes
with ten calls each. Each of three fixture scenarios has fetching on/off and
diagnostics on/off, ten observations per cell. Conditions are shuffled with seed
80 each CLI round; retained conditions run in batches. The empty and unfinished
cases do not actually fetch even with fetching enabled. Only the `page` condition
measures fetching: a completed Bing result, a hanging Yahoo response, and a local
page delayed 50 ms. Concurrency one exercises provider queueing. Result minimum
five prevents the single completed fixture result from stopping the search early.
Ranking and caching are disabled. Fixtures use loopback HTTP; client construction
still initializes the same TLS/proxy machinery as normal searches. Ambient proxy
variables are removed for fixture trials and loopback bypass is explicit.

Diagnostics-on means provider traces plus existing CLI benchmark artifacts for
CLI runs, and provider traces for library runs. Diagnostics-off means neither
capture environment variable is set. Existing `initialize`, `search`, `fetch`
artifact timings and lifecycle phase/cancellation records are retained rather
than replaced. The observer fills lifecycle gaps such as runtime construction,
CLI parsing, client components, drop handling and output boundaries.

No harness build or test workload overlapped these measurements. This is a
desktop host, not an isolated performance machine: unrelated scheduling and
system activity remain uncontrolled, and large outliers are retained.

## Controlled distributions

Milliseconds; each row has n=10. The final column is both nearest-rank p95 and
maximum at this sample size. It must not be treated as a well-estimated tail.

| Scenario | Fetch enabled | Diagnostics | Process min | Process median | Process p95/max | Search-stage median |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Empty | no | off | 1130.715 | 1155.938 | 1201.843 | 1001.966 |
| Empty | no | on | 1126.921 | 1151.026 | 1280.770 | 1002.363 |
| Empty | yes | off | 1123.193 | 1149.526 | 1288.817 | 1002.278 |
| Empty | yes | on | 1125.483 | 1154.494 | 1225.904 | 1003.231 |
| Unfinished/queued | no | off | 1126.873 | 1149.583 | 1224.634 | 1002.345 |
| Unfinished/queued | no | on | 1135.391 | 1152.114 | 1254.437 | 1003.378 |
| Unfinished/queued | yes | off | 1129.487 | 1160.441 | 1543.831 | 1002.463 |
| Unfinished/queued | yes | on | 1129.067 | 1152.237 | 1222.010 | 1003.519 |
| One page | no | off | 1125.761 | 1148.291 | 1237.464 | 1002.254 |
| One page | no | on | 1124.362 | 1141.969 | 1363.730 | 1002.599 |
| One page | yes | off | 1178.662 | 1202.113 | 1588.251 | 1002.546 |
| One page | yes | on | 1190.317 | 1207.914 | 1559.791 | 1002.972 |

All fixture outcomes passed semantic validation: unfinished/queued CLI searches
exit unsuccessfully, explicit-empty searches return successful empty envelopes,
and page searches return one result with fixture evidence when fetching is on.

Pooling the six CLI cells within each diagnostics setting gives 60 observations
per setting (120 provider-drop observations). These aggregates describe this
balanced experiment, not a production query distribution:

| Component | Off median | Off max | On median | On max |
| --- | ---: | ---: | ---: | ---: |
| Client initialization | 133.349 | 309.165 | 134.134 | 237.799 |
| Standard search client | 5.851 | 26.020 | 5.894 | 63.981 |
| Yahoo client | 128.218 | 282.754 | 126.820 | 230.853 |
| Fetch client build | 0.211 | 12.348 | 0.172 | 0.932 |
| Search return beyond budget | 2.294 | 13.668 | 2.900 | 9.259 |
| Provider drop, per job | 0.005 | 0.035 | 0.233 | 3.177 |
| Runtime shutdown | 0.343 | 12.908 | 0.386 | 5.192 |
| Outside-main observation residual | 11.080 | 296.149 | 11.039 | 320.775 |

Component medians do not add up to median totals. Some large process outliers
are outside the observed Rust-main interval, so attributing every tail to client
construction would also be wrong. Synchronous persistence costs are visible, but
the data do not justify attributing the original roughly 140 ms to cleanup.

## One reconciled timeline

An explicit-empty, no-fetch, diagnostics-off trial took **1132.714 ms externally**
and reported **1123.447 ms** in JSON. Times below are offsets from the observer's
Rust-main origin, rounded to microsecond precision:

| Event | Offset, ms |
| --- | ---: |
| Rust main entry | 0.000 |
| Runtime ready | 0.201 |
| CLI parsing begins / ends | 0.206 / 0.365 |
| Search handler enters | 0.365 |
| Client construction begins | 0.378 |
| Standard client begins / ends | 0.394 / 4.176 |
| Yahoo client begins / ends | 4.177 / 121.112 |
| Fetch client begins / ends | 121.124 / 121.252 |
| Client construction ends | 121.255 |
| Shared provider deadline established | 121.276 |
| Nominal deadline | 1121.276 |
| Final provider drop begins / ends | 1123.199 / 1123.209 |
| Collector returns | 1123.222 |
| Search API returns to CLI | 1123.800 |
| JSON serialization begins | 1123.813 |
| Result output completes | 1123.827 |
| Handler returns, including client destruction | 1123.898 |
| Runtime destruction completes | 1124.249 |

The observed main→runtime-destroyed interval is 1124.249 ms. Subtracting it from
external launch→reap leaves **8.464 ms** outside that interval. This residual
includes loader/startup before Rust main, the observer's final emission, process
exit and parent subprocess bookkeeping. It is explicitly **not** assigned wholly
to startup or wholly to shutdown. The two clocks' origins are never subtracted
from one another. JSON measures handler entry to its pre-serialization sample,
so it includes client initialization and excludes serialization/output/shutdown.

The deadline marker is immediately after deadline construction; its timestamp
has small positive observer skew. Post-deadline collector time includes scheduler
wakeup, final future/drop work, and collection. The existing lifecycle intervals
are integer milliseconds; new marks use fractional `Instant` durations.
Python reports a monotonic `mach_absolute_time()` clock with approximately
41.7 ns nominal resolution, which is not scheduler precision.

## Retained clients and root-store control

Each retained condition constructs one client before ten searches. Subsequent
calls (n=9 per cell) have median search API time between 1001.914 and 1004.086 ms;
first calls range from 1001.570 to 1004.547 ms. There is no approximately 130 ms
penalty in the first search after construction. Client reuse avoids paying setup
again; these results do not establish a network warm-up improvement.

In the page/fetch condition, retained page fetching takes 55.680 ms median with
diagnostics off and 56.135 ms on (n=10 each), separately after the search stage.
Retained batch initialization itself varies from 114.289 to 239.378 ms.

The root-store control alternates ten normal and ten preload fresh processes,
each constructing/dropping ten clients. It invokes the existing public primp
`tls::default_root_store_arc()` in the preload arm. The locked primp 2.0.0 source
caches bundled plus native certificates with `OnceLock`; no trust configuration
is removed or substituted.

| Measurement | n | Min | Median | Max |
| --- | ---: | ---: | ---: | ---: |
| Normal first client construction | 10 | 114.206 | 187.479 | 262.312 |
| Preloaded default root store | 10 | 113.358 | 155.622 | 273.858 |
| First client after preload | 10 | 2.589 | 4.098 | 23.053 |

Subsequent client construction medians are 0.716 ms (normal) and 0.809 ms
(preload), n=90 each. This intervention isolates root-store initialization as the
major one-time component. It does not separately time native certificate
enumeration versus parsing/insertion, or prove identical costs on other systems.
Proxy discovery and the remaining client setup stay in the measured constructor;
they are not network request time. Normal/preload medians should not be
subtracted to estimate an exact per-process optimization under desktop variance.

## Observer and diagnostics limitations

An interleaved one-nanosecond-budget control compares the unmodified CLI with the
observer build in fresh processes, n=20 per binary per fetch/trace condition
(160 processes total). It expires before provider work and needs no endpoint
override in the unmodified binary. Observer-minus-original process median
differences are +0.693, +0.138, +1.448 and +0.098 ms across the four conditions.
This estimates the practical observer/build effect in that short-path experiment;
it is not an exact calibration to subtract from network trials. Allocation,
locking, explicit runtime code generation and the final stderr emission are
included. No statistically precise sub-millisecond overhead claim is made.

Trace files reveal a deadline-boundary subtlety: in all 20 traced fresh
unfinished/queued trials, Bing is cancelled during body reading, while Yahoo
records about one second in queue followed by a censored application send at
expiry. Releasing Bing's semaphore permit permits the queued future to advance
before the timeout finishes polling it. The recorder counts an application send,
not a wire request; these traces cannot prove post-deadline bytes were sent.
This does not explain the initialization excess. Do not describe this fixture as
proving that queued jobs always have zero sends.

The experiment uses small parser inputs and no cache. It does not clear
[parser cancellation #14](https://github.com/rafaelpierre/kestrel-rs/issues/14)
or [cached-fetch budget #18](https://github.com/rafaelpierre/kestrel-rs/issues/18).
The measured synchronous drop persistence remains relevant to
[#17](https://github.com/rafaelpierre/kestrel-rs/issues/17), particularly with
slower disks or more providers. Public completion reporting remains owned by
[#81](https://github.com/rafaelpierre/kestrel-rs/issues/81).

## Labeled live reproduction

Forty additional sequential CLI processes queried `"Sonic Youth" "tunings"`
with all default providers, `--search-budget 1 --min-results 1 --top-k 1`, JSON
output and no ranking, alternating fetch/trace conditions. This follows one of
#76's historical queries, using the current binary above rather than claiming
identity with the September 12 binary. Each cell has n=10; timings are ms.

| Fetch | Diagnostics | Empty / one result | Process min | Process median | Process p95/max | Initialization median | Search median |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| no | off | 7 / 3 | 438.886 | 1584.160 | 2070.225 | 279.804 | 1001.475 |
| no | on | 8 / 2 | 445.893 | 1597.732 | 1749.820 | 293.757 | 1002.549 |
| yes | off | 7 / 3 | 1557.326 | 1674.550 | 10535.440 | 286.757 | 1001.939 |
| yes | on | 7 / 3 | 1219.442 | 1555.807 | 10810.291 | 238.512 | 1002.478 |

All processes exited successfully, including empty outputs. The provider stage
never exceeded 1005.786 ms, and sometimes stopped early after a qualifying
result. Actual fetch phases occur in only three runs per fetched cell. Without
traces they lasted 10003.720–10006.841 ms; with traces, 131.967–10003.244 ms.
These include the separate default ten-second page timeout, not a search-budget
extension or a guarantee that returned candidates have fetched evidence.

The original exact process duration is not stable: this later live batch has
higher initialization costs and an outside-main residual around 310–312 ms
median across cells. Runtime shutdown medians are only 0.698–0.846 ms. One
no-fetch/no-trace run spends 142.802 ms in runtime shutdown; the other cell
maxima are at most 4.524 ms. Consequently, assigning the larger external
remainder to runtime shutdown, DNS, or parser cleanup would be unsupported.
No claim about provider coverage or the cause of #76's filtering follows from
this timing experiment.

A supplemental ten-process discovery batch (five per trace setting) added a
parent timestamp immediately after `Popen` returned. Spawn-call durations were
1.195–3.038 ms and the outside-main remainder was 7.534–11.156 ms; the earlier
approximately 300 ms remainder did not recur. The final runner retains this
additional timestamp. This narrows normal launch-call overhead but leaves the
earlier intermittent startup/exit/parent tail unattributed; do not retroactively
subtract the supplemental measurements from the original batch.

## Contract recommendation

The existing search budget starts in provider orchestration, after CLI parsing
and client construction. It includes provider queueing/retries and is followed
by optional fetching/ranking, serialization, output and destruction. A finite
amount of cooperative timeout/cleanup overhead is observable. This experiment
does not establish a roughly 140 ms provider deadline violation or a hard
resource-release guarantee.

A separate end-to-end deadline could serve callers with a strict outer latency
allowance, but needs a separate contract decision covering initialization,
fetching, partial results, output and blocking work. Wrapping only the search
future would not satisfy that use case. Callers requiring a hard process cutoff
need external process supervision today. Do not rename `--search-budget` or
silently subtract initialization from it as part of this investigation.

The shipping CLI, library, generated skill and timing schemas are unchanged;
there is no new public behavior requiring a skill migration. Raw generated
artifacts and live page captures remain ignored locally. The report and harness
are the reviewable evidence committed with this investigation.

## Validation

- Repository `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, and `cargo build --release` passed.
- The final prepared benchmark source copy passed formatting, the same clippy
  command and all-feature tests: 167 passed, four existing tests ignored.
- All three benchmark Rust programs built in release mode. The Python fixture,
  invalid-outcome rejection and percentile tests passed (three tests).
- Controlled summary assertions verified all 120 CLI outcomes and 120 retained
  calls. The observer comparison and live records were summarized separately.
- No production API, CLI behavior, skill generator, dependency, or MSRV change;
  temporary skill installation is not needed for this benchmark/report-only PR.
