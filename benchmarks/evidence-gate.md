# Canonical evidence gate

Issue [#148](https://github.com/rafaelpierre/kestrel-rs/issues/148) separates
repeatable evidence capture from provider/extraction fixes and semantic judgment.
`evidence_gate.py` is a standard-library Python entry point for the current
[ten-question dataset](codex-search-2026-09-11/queries.json). The old
`codex-search-2026-09-11/run-kestrel.py` remains a historical timing helper.

## Declare and freeze a run

Read `POLICY` in the runner before execution. It declares metadata-only discovery
with all default providers, top-k/collection minimum 20, a 10-second search budget,
a 15-second direct-fetch timeout, 100,000 retained characters, 4,000,000 response
bytes and a 30-second outer bound for either command. Each question permits two
searches and three fetches. Cache is disabled; upstream state is uncontrolled.
Remote telemetry is disabled to keep raw evidence local. Provider traces and
benchmark artifacts are saved inside each attempt. No production CLI semantics,
provider configuration, generated skill or dataset is changed by this runner.

```sh
python3 benchmarks/evidence_gate.py /tmp/kestrel-gate-window-1 init --assessor 'model/version or human name'
cat /tmp/kestrel-gate-window-1/SKILL.md
python3 benchmarks/evidence_gate.py /tmp/kestrel-gate-window-1 search
```

Use a fresh directory outside the checkout. Initialization builds the executable
with `cargo build --release --locked` in this worktree and saves build output,
revision/HEAD tree, file content hashes and modes (including new unignored files),
binary path/version/hash, dataset, generated skill, rubric and assessor. The skill
is installed into a new temporary project; the CLI records that temporary path in
the user's installation registry. Existing installed skills are not overwritten.
Read the saved skill before discovery. Source, binary, policy, rubric, dataset or
skill drift blocks further retrieval/assessment: start another complete run.
A run manifest appears only after successful initialization. Failed initialization
remains retained and cannot be reused.

`search` with no ID executes each initial exact manifest query sequentially. Read
all returned candidates, complete snippets, diagnostics and relevant artifacts.
Discovery success and extraction success do not establish answer support.

```sh
python3 benchmarks/evidence_gate.py /tmp/kestrel-gate-window-1 search q04 \
  --query 'site:postgresql.org EXPLAIN ANALYZE BUFFERS documentation' \
  --reason 'Initial candidates lacked the requested official explanation'
python3 benchmarks/evidence_gate.py /tmp/kestrel-gate-window-1 fetch q04 'URL_FROM_THIS_RUN' \
  --reason 'Selected official documentation candidate for execution and buffer semantics'
```

The first search cannot be rewritten. Recovery is optional and requires a reason
recorded before execution; retain keyword FTS form, entities, versions, dates and
site constraints. The runner enforces declared manifest site tokens; the assessor
must check semantic preservation. Fetch URLs must have appeared in this question's
returned results, not another run or remembered source list. Refetches consume a
fetch slot. This first version does not support linked-page traversal or importing
redirect evidence. Kestrel follows redirects but does not expose the chain; do not
claim a required official migration from success or the `Source:` wrapper alone.
Record that gap as a blocker. Source-selection improvements belong to #79;
provider fidelity to #32; extraction regressions to #22; broader comparisons to #84.

## Assess retained evidence

Create one judgment JSON per question with these fields:

```json
{
  "verdict": "pass",
  "layer": "none",
  "answer": "A complete answer supported by the cited passages.",
  "rationale": "Explain how every question-specific minimum is met.",
  "inspection": "Describe all candidates inspected and why sources were chosen or rejected.",
  "fetch_disposition": "Record successful/failed extractions and why remaining fetches were skipped.",
  "citations": [
    {"attempt": "fetch-1", "url": "URL_FROM_THIS_RUN", "passage": "Exact nonempty text from saved content"}
  ]
}
```

Allowed failure layers: `upstream`, `constraints`, `collection`, `selection`,
`extraction`, `assessment`; use `none` for a supported answer. A failure can retain
useful partial evidence; write an explicit abstention when no answer is supported.
Citations to `search-1`/`search-2` match snippets; fetch citations match complete
retained content. Passage matching guards provenance only: a manual assessor
must still enforce official domains, dates, versions, uncertainty, all answer
minima and citation support. Never submit the illustrative JSON as a judgment.

```sh
python3 benchmarks/evidence_gate.py /tmp/kestrel-gate-window-1 assess q04 /tmp/q04-judgment.json
python3 benchmarks/evidence_gate.py /tmp/kestrel-gate-window-1 report > /tmp/window-1-report.json
```

Each assessment is exclusive-create and closes that question to further calls.
Missing judgments fail. There is no automated semantic pass detector. Save
corrections separately and rerun rather than overwriting evidence. Directories
reserve attempts, including interrupted ones. Receipts record full argv, UTC
start/end, monotonic wall time, exit code, timeout flag and stdout/stderr hashes.
Output is streamed to files, preserving partial timeout output. A hard interruption
can leave no receipt: timing stays unknown and the gate cannot pass. Do not delete
an interrupted attempt to reclaim budget. Do not concurrently operate on one run;
exclusive creation rejects competing reservations rather than serializing them.

The report sums discovery, fetch and total subprocess times separately, including
recoveries/errors. These are tool times, not assessor latency. No percentile or
performance claim is made. Keep full artifacts local; publish a sanitized ten-row
summary with actual answers, selected URLs/passages, layers, verdicts, timings,
identity, exact policy and limitations in the PR. Do not publish whole page bodies
or raw traces. Run three separately identified full windows with the same policy
and retain every result; report each window independently, never union successful
rows across windows. An unchanged-input rerun measures live variability, not a
causal improvement. A failed question keeps the PR draft and #148 open.

## Deterministic checks

```sh
python3 -m unittest discover -s benchmarks -p test_evidence_gate.py
```

Tests cover timeout retention, immutable attempts, pre-launch interruption accounting,
missing/duplicate/extra dataset questions, interrupted-call accounting,
unknown source URLs, rewritten first queries, lost site restrictions, fabricated
passages, changed evidence and missing judgments. Ordinary CI discovers these
standard-library tests alongside existing benchmark tests. They do not certify
live retrieval or semantic answer quality.
