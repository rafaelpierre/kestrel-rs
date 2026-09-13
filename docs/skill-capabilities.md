# Generated skill capability audit

Checked against the implementation for issue #82. The installed reference comes
from [generate_skill_md](../src/skill.rs), with live Clap search/fetch help from
[cli.rs](../src/cli.rs). CLI defaults and output schemas are unchanged. This is
workflow guidance using existing primitives, not evidence of a universal speed or
answer-quality advantage; [#84](https://github.com/rafaelpierre/kestrel-rs/issues/84)
owns the comparative workflow evaluation.

| Implementation/source | CLI exposure | Generated skill coverage and owner/defer decision |
| --- | --- | --- |
| Query planning, filters and engines: [query.rs](../src/query.rs), [cli.rs](../src/cli.rs) | Query, repeated query/engine, syntax, region and recency flags | Checked: live help plus constraints and recovery guidance; preserve phrases/Boolean intent. |
| Collection/stopping: [search.rs](../src/search.rs) | Minimum, search budget/concurrency, fanout, ignored quorum | Checked: separate collection/output limits and stage deadlines; policy measurements remain #75. |
| Fetch selection and ranking: [cli.rs](../src/cli.rs), [ranking.rs](../src/ranking.rs) | Fetch switches, candidate cap, pre-rank and ranking policies | Checked: live conflicts, existing recipes and selective reading; hidden hybrid components are not JSON fields. Evaluation remains #78. |
| Page limits and caching: [fetcher.rs](../src/fetcher.rs), [cache.rs](../src/cache.rs) | Search page controls/cache; standalone fetch URL and limits | Checked: character/byte caps, failures, truncation and cache exclusions; standalone fetch has no cache. Persistent search recovery remains #70. |
| Result serialization: [model.rs](../src/model.rs), [cli.rs](../src/cli.rs) | Search results/elapsed envelope; fetch URL/content/elapsed | Checked: existing schemas and null content; success does not establish usefulness. Default bounded completion/evidence diagnostics and --no-diagnostics are documented; see [contract](structured-diagnostics.md). |
| Skill installation: [cli.rs](../src/cli.rs), [config.rs](../src/config.rs) | `skill install --agent --scope --force` | Checked: binary identity, exact target refresh, agent aliases and project/global paths. Avoid unrelated installs. Issue #25 adds config lock/timeout, atomic writes, and failure/compatibility guidance; see [installation state](installation-state.md). |
| Binary self-install: [install.rs](../src/install.rs) | `install`, exclusive `--system` / `--dir` | Added: copying versus updating, destinations, replacement/no-op and PATH/platform limits. |
| Skill removal: [cli.rs](../src/cli.rs) | `skill uninstall`, interactive selection | Added: recorded paths, default-all selection, stale records, no invented switches; does not remove binary. |
| Provider lifecycle and raw capture: [benchmarking.rs](../src/benchmarking.rs), [provider_diagnostics.rs](../src/provider_diagnostics.rs) | `KESTRELSEARCH_PROVIDER_TRACE_DIR`, environment-accessible, no flag | Added: optional capture recipe, correlation, cancellation/censoring, missing bodies and redaction; persistence changes remain #17. |
| Search artifacts: [benchmarking.rs](../src/benchmarking.rs), [cli.rs](../src/cli.rs) | Both benchmark directory and run-ID environment variables required | Added: file location, phase/report snapshots, different schema, search-only boundary. #75/#78/#80 reuse this interface. |
| Local events: [logging.rs](../src/logging.rs) and search/fetch call sites | Automatic best-effort side effect; no disable/location flag | Added: UTC daily path, selected event coverage, privacy and incomplete/resume limitations. |
| Transport and retained client: [client.rs](../src/client.rs), [transport.rs](../src/transport.rs), [warmup.rs](../src/warmup.rs) | Within-invocation pools; tuning/warm-up are library-only | Checked: explain separate CLI initialization. No invented persistent session/warm-up flags; #80/#84 own measurements, #1 MCP. |
| Batch/cached direct APIs: [client.rs](../src/client.rs) | Search uses batch fetch internally; direct fetch takes one URL | Checked: selective reads use individual fetches. No batch/cache flags invented. |
| Offline rank replay: [rank_replay.rs](../examples/rank_replay.rs) | Cargo example, not a CLI subcommand | Defer specialized experiment guidance to #78; existing CLI ranking policies documented. |
| Streaming probes: [probe.rs](../src/search/streaming/probe.rs) | Test-only, not ordinary diagnostics | Defer research to #29/#46; no production observability claim. |
| Advisory content quality: [content_quality.rs](../src/content_quality.rs) | Library assessment methods; opt-in search artifact assessments; default CLI diagnostics, with --no-diagnostics opt-out | Whole-message shell detection and bounded first-article recovery are documented in the generated skill and [quality policy](content-quality.md). No useful-evidence certification, rejection or ranking penalty. |
| Automated search recovery, find/passages | No current CLI capability | #79/#83 own implementations. Workflow recovery is an agent decision, not a new command. |

## Validation and task review

Generated Bash command examples are parsed against the current command tree,
including the environment-prefixed diagnostic recipe. The CLI integration suite
installs into a temporary project with an isolated home, compares live search/fetch
help, refreshes that installation, and exercises removal. A separate workflow test
executes the installed skill's direct-reading recipe against local HTML fixtures,
parses JSON and checks usable text, empty extraction, truncation notices and
optional header capture. Existing provider diagnostic tests exercise raw responses,
lifecycle records and cancellation with local fixtures. No live provider outcome
or retrieval-quality improvement is claimed.

Task review:

- Known URL: direct fetch; discovery is skipped.
- Uncertain discovery: retain alternatives, inspect source quality, spend a bounded
  remaining call on explicit reformulation or expanded collection.
- Quotation: inspect page text and context; metadata match alone is insufficient.
- Empty results: report the limited retrieval outcome, not nonexistence of sources;
  do not silently relax constraints.
- Unusable body: try another source or a bounded larger-prefix refetch; report
  uncertainty when the task budget is exhausted. No semantic usefulness classifier or find feature assumed.

The generated skill is self-contained because installation currently writes only
`SKILL.md`; this repository audit is a maintainer reference, not a runtime dependency.
