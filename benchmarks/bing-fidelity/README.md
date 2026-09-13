# Bing fidelity experiments

Run from the repository root. These tools are opt-in experiments, not production
relevance filtering. The Rust harness is an ignored unit test so it exercises the
same private request builder, parser and orchestration functions as production
without adding a public configuration API.

```sh
python3 benchmarks/bing_fidelity.py run \
  --output benchmarks/results/bing-fidelity-new \
  --windows 2 --interval 300
```

The runner compiles once and freezes the test executable for the entire schedule.
The schedule records its SHA-256 and the query manifest hash. Every window writes
`runs.json` incrementally and marks completion only after all configured searches
finish. An existing output directory is rejected. Windows run sequentially; if a
window exceeds its interval, the next begins immediately after it completes.
There is no background automation after the command exits.

The default matrix uses six declared query intents, Chrome 146/macOS/en-GB
headers, no explicit region, a five-second search budget and reused clients
without a cookie jar. The profile is fixed for reproducibility; production CLI
profiles can vary. It alternates variant order across queries:

- `bing-standard`: one complete raw Bing response, before query filtering.
- `fanout-native-min5`: all nine providers, native query syntax, five-result minimum.
- `fanout-portable-min5`: all nine providers, portable syntax, five-result minimum.
- `fanout-portable-min20`: the same portable search with a twenty-result minimum.

These are streaming result minima, not output caps or guarantees of provider
diversity. A minimum of twenty is still bounded early stopping, not full fanout.
All fanout arms use concurrency ten, production retries and the same five-second
budget. Page fetching/ranking is excluded. Provider quorum is unset.

Each successful isolated response also stores native and portable views computed
from the exact same original entries using production normalization, before URL
sanitization. The scorer expands these into `*-native-replay` and
`*-portable-replay` groups. Failed captures contribute empty/error observations
to both groups too. These paired views make no extra requests and have no
independent latency; replay p50/p95 and attempt estimates are deliberately null
(or zero for additional isolated attempts). Compare the raw entries to these
views to inspect filter losses independently of upstream time variation.

Use `--transports` for a separate four-variant matrix: standard Bing, browser
impersonation, explicit `gb-en` region, and the previously observed browser
`form=QBRE` parameter. Other than the named variation, the budget/profile/query
are matched. This does not reproduce the historical three-second experiments.

Use `--encodings` for a separate two-variant experiment comparing `+` and `%20`
space encoding while preserving the decoded query. Optional `--raw` retains local
unredacted HTML in the ignored output directory. It is disabled by default.
Sanitized JSON retains public query/result text, response hashes, HTTP status and
version, selected cache headers and elapsed time. Cookies and authorization are
not captured. Result URL query strings/fragments are removed; this can collapse
semantically distinct URLs, so judgments also use the full title and snippet.
Initial URLs preserve exact query encoding; final URLs retain only q/cc. Redirect
chains and browser network equivalence are explicitly unavailable, not inferred
from final URLs. The harness does not bypass
system proxies or browser security checks.

## Relevance judgments and scoring

```sh
python3 benchmarks/bing_fidelity.py template \
  benchmarks/results/bing-fidelity-new/window-*/runs.json \
  --output benchmarks/results/bing-fidelity-new/judgments.json
# Review every entry against queries.json; set relevant true/false and a reason.
python3 benchmarks/bing_fidelity.py score \
  benchmarks/results/bing-fidelity-new/window-*/runs.json \
  --judgments benchmarks/results/bing-fidelity-new/judgments.json \
  --output benchmarks/results/bing-fidelity-new/scores.json
```

Unjudged entries remain null. No word-overlap heuristic supplies relevance labels.
Judgments are specific to query + destination + title + snippet. Conditional
precision@5 divides relevant top-five slots by five times the number of nonempty
searches (missing slots count as nonrelevant). Usable coverage divides searches
with at least one relevant top-five result by **all** scheduled searches, including
empty/error/deadline outcomes. Missing judgments suppress exact metrics where
necessary and produce coverage bounds. With no nonempty searches, conditional
precision is null. Latency uses nearest-rank percentiles across all searches.
Public provider diagnostics do not establish send counts for queued/cancelled
work, so the current matrix reports no fanout attempt estimate. Historical
artifacts retain the old scoring field; do not treat that estimate as wire
requests. Isolated searches explicitly make one application attempt each. The
scorer checks an adjacent schedule.json: an incomplete or failed schedule is rejected even when an earlier window finished. Every scheduled window must be
supplied exactly once; missing, unexpected and duplicate inputs (including symlink
aliases) are rejected. When combining schedules, each must be complete. When copying evidence, retain
the parent schedule and all windows; absence of a parent schedule permits legacy
standalone artifacts and cannot prove the full schedule was supplied.

The raw-response excerpt tool preserves the first two organic blocks and their
semantic markup for parser regressions:

```sh
python3 benchmarks/sanitize_bing_fixture.py path/to/local-response.html \
  tests/fixtures/providers/bing-new-excerpt.html
```

It removes active content, nonessential attributes and URL tracking, and prints
the raw-response hash. Review the retained text before sharing; sanitization does
not remove sensitive words inside page text. Record timestamp, request context
and hash in fixture provenance. Excerpts are not byte-identical raw responses.

## Independent-environment comparison

Use the same reviewed source revision and query manifest in a separate checkout.
Run the normal deterministic tests first, then execute this opt-in command on
each machine/network, with distinct non-sensitive labels and fresh directories:

```sh
python3 benchmarks/bing_fidelity.py run \
  --output benchmarks/results/bing-fidelity-environment-b \
  --environment environment-b --windows 2 --interval 60
```

The runner stores the source commit, tracked-diff hash, frozen test executable
hash, manifest hash, OS/architecture, label and proxy-variable **presence only**.
Cross-platform executable hashes may differ; source/configuration/manifest must
match. The label and absent proxy variables do not establish independent egress:
system proxies, VPNs and intermediaries are not determined by this tool. Do not
publish addresses, credentials, cookies or session identifiers as proof.

For each query, navigate a normal browser to the exact `initial_url` from the
isolated row near the adapter run, and record UTC times, sanitized final q/cc URL,
rendered organic titles/snippets/destinations, and whether the view changes after
navigation. Do not conflate an initial loading state with final rendered results.
Record browser locale/profile differences and use independently controlled
session/network evidence if available. DOM captures alone do not establish HTTP
response contents, redirect chains, JavaScript causality or egress equivalence.
Do not disable TLS verification, bypass a challenge, or infer cache causality
from temporal improvement.

Keep all windows and null judgments; reuse judgments only when the complete
query + URL + title + snippet key matches. Report per-environment and per-window
results before aggregating. No generic token threshold supplies judgments.
`--raw` is optional and local; share only reviewed, sanitized evidence.

See `docs/issue-32-bing-fidelity-20260913.md` for the current baseline and remaining
gaps. `docs/issue-32-bing-fidelity.md` preserves the historical investigation.
