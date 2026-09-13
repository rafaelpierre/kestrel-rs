# Native query default regression validation (#141)

The CLI, `SearchOptions::default()` and single-provider APIs now pass query text
through without portable lexical rejection. Portable filtering remains explicit.
Shell quoting groups an argument; it does not impose AND or phrase matching.
Native hostname restrictions and URL validation remain. This intentionally changes
the v3/v4 default introduced in #51 / PR #54. It does not fix upstream retrieval
(#32), promise relevance, or guarantee a subsecond search.

## Deterministic checks

`cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features`, `cargo build --release` and `git diff --check` passed.
The suite passed 211 tests; five existing opt-in tests were ignored. Coverage
includes shell argument/literal quote preservation, metadata without query terms,
native defaults, explicit portable rejection and malformed syntax, URL/site checks,
and streaming acceptance cancelling unfinished providers at the unique minimum.
Existing provider serialization, deduplication, budget, fetch and ranking tests
also passed. No dependency or MSRV change.

The first test pass found the old installed-skill default assertion, which was
updated to require native help plus explicit portable guidance. The full suite
then passed. The release binary installed its skill into a temporary project;
the generated prose, native help default and explicit portable fetch-score recipe
were checked. The temporary installation was removed without changing installed
user skills. `--min-fetch-score` still requires portable mode and now needs that
mode explicitly; the help, examples and rejection test cover this interaction.

## Ten-question live replay

Input is the user's September 13 ten-question test and its nine recorded recovery
queries, copied unchanged into `benchmarks/query-defaults`. One sequential pass
alternated which policy ran first for each pair. Both arms used the same release
binary, metadata-only output, top-k 5, minimum 10 and a five-second search budget.
The default arm omitted query syntax; the control explicitly selected portable.
Benchmark diagnostic artifacts were enabled. No external retrieval was used.

This is a **fixed replay of the historical recovery queries**, not a fresh adaptive
agent trial. All nine recovery queries were replayed in each arm, including where
initial evidence was already available. Fetch selection was manual after inspecting
all metadata, used only URLs returned by the respective arm, and stayed within
three fetches per question, 40,000 characters, 2 MB and 15 seconds.

| Question (exact initial query) | Default counts (initial, recovery) | Portable counts |
| --- | --- | --- |
| What is the capital of Australia? | 5 | 3 |
| why is the sky blue Rayleigh scattering | 5, 5 | 0, 1 |
| Python asyncio TaskGroup exception handling ExceptionGroup | 5, 0 | 0, 0 |
| site:postgresql.org EXPLAIN ANALYZE BUFFERS | 0, 0 | 0, 5 |
| site:grafana.com Tempo TraceQL parent child spans | 0, 5 | 0, 4 |
| site:learn.chatgpt.com Codex otel trace_exporter | 1, 2 | 0, 0 |
| Rust E0382 use of moved value how to fix | 5, 5 | 0, 0 |
| James Webb TRAPPIST-1 b atmosphere observations 2025 | 5, 5 | 0, 0 |
| Grafana Tempo 3.0 release notes breaking changes | 5, 5 | 0, 0 |
| London Science Museum opening hours | 5, 5 | 0, 0 |

Default: 15/19 nonempty searches; median process time **0.593 s**.
Portable: 4/19 nonempty searches; median **5.152 s**. All 38 searches exited zero.
There were 14 direct fetch attempts, 13 with successful exit status. The PostgreSQL
message fetch failed. Successful fetch exit is not proof of answer quality.

Evidence inspection found:

- Both modes supplied fetched support for Canberra and Rayleigh scattering (the
  latter via recovery). Unrelated initial default results for “why” did not count
  as evidence; the useful recovery result was selected below rank one.
- Default TaskGroup results supplied secondary material, but sources conflicted
  about exception handling and the official-documentation recovery was empty.
  Treat precise behavior as unverified, not a complete supported answer.
- Default TraceQL pages supplied introductory/structural context but incomplete
  operator evidence; portable recovery supplied Grafana's structural-operator
  release article. This live window does not show default dominance.
- Default Codex pages supplied general telemetry configuration context but did
  not establish the requested `trace_exporter` details in extracted text.
- Default supplied NASA's TRAPPIST-1 overview with December 2025 context and
  evidence against a thick atmosphere for b. Other returned pages concerned
  different planets/years and were not interchangeable evidence.
- E0382, Tempo 3.0 and museum-hours results were unrelated or insufficient even
  after recovery. Default PostgreSQL searches were empty; portable retrieved
  mailing-list results, but the selected fetch failed. Abstain for those tasks.

These are evidence observations, not a calibrated answer-accuracy score. Faster
nonempty responses can still be wrong. In particular, Bing returned dictionaries,
games, generic city pages and unrelated businesses for several complete queries.
Removing the local filter does not repair that known provider problem (#32).
Portable PostgreSQL recovery succeeded where default did not; alternating live
requests do not freeze provider responses, so this cannot establish a causal
policy loss or recovery. One pass cannot establish stable coverage or latency.
The historical 1/10 assessment used a different live window; no causal improvement
percentage is claimed against it.

## Reproduction and artifacts

```sh
python3 benchmarks/query-defaults/run.py --binary target/release/kestrel --output /tmp/query-default-replay --stage initial
python3 benchmarks/query-defaults/run.py --binary target/release/kestrel --output /tmp/query-default-replay --stage recovery
```

The runner refuses an existing initial output directory and a repeated recovery
stage, records binary identity, arguments, outputs and process times, and retains
candidate diagnostics. Fetch/evidence assessment remains manual. Python syntax
compilation passed. Generated captures remain untracked.

Local evidence: `/tmp/kestrel-141-live/{metadata.json,calls.jsonl,fetches.jsonl,summary.json}`
and per-call candidate artifacts. Binary SHA-256:
`1b451db8a70c77ce15bbba14a5092679f87e5424cced706607ea9ac4290dd93e`.
Source base: `bb40b37`, with this PR's tested source changes; the binary's version
string remains 4.1.0 because release-plz owns version changes. It is not claimed
to be the installed 4.1.0 release binary. The user's installed binary is unchanged.
