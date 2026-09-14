# Complete-answer workflow evaluation: issue #84

Status: **protocol/evaluator preparation; no completed agent trials and no workflow
recommendation**. This staged deliverable implements the September 14 triage in
[#84](https://github.com/rafaelpierre/kestrel-rs/issues/84). Full trials remain
blocked until the retrieval-readiness and isolation prerequisites below are met.
No production defaults, generated-skill behavior, or historical results change.

## Prepared experiment

`workflow_eval.py` uses only the Python standard library. It prepares six
counterbalanced rounds for each question in the existing
[30-question coding pilot](coding-pilot-20260911/README.md), strips undeclared task
fields, exports single-question packets, and validates imported trajectories and
separate semantic judgments. It **does not launch agents**, enforce filesystem
isolation, certify semantic truth, or claim an experiment is ready. The trial
execution adapter remains follow-up work within #84, not a hidden fallback to
unrestricted agents running inside this checkout.

```sh
python3 benchmarks/workflow_eval.py prepare \
  benchmarks/coding-pilot-20260911/questions.json \
  --model gpt-5.3-codex-spark > /tmp/workflow84-plan.json
python3 benchmarks/workflow_eval.py packet /tmp/workflow84-plan.json \
  coding-01 selective > /tmp/workflow84-question.json
python3 -m unittest discover -s benchmarks -p test_workflow_eval.py
python3 benchmarks/workflow_eval.py validate /path/to/trajectory.json \
  --judgment /path/to/independent-judgment.json
```

The requested smallest available model is the intended first choice; pin its
resolved version at execution and record access verification. The example names
the smallest model advertised in this session, not a verified benchmark run.
Preparation and synthetic tests make **zero model calls**. No automatic upgrade
is allowed on failure. Record unsupported sampling controls as null, retain actual
provider defaults and use identical settings for every arm. An unresolved alias
or different model between arms invalidates comparisons.

## Treatments and controls

| Arm | Discovery | Follow-up reading |
| --- | --- | --- |
| Bundled | Search fetches/ranks up to three pages | Direct reads for missing details within common page budget |
| Selective | Metadata search, then agent selection | Individual direct fetches |
| Adaptive | Agent chooses bundled or metadata per call | Selective direct reads and bounded recovery |

For supplied URLs, all arms may fetch immediately. These strata may converge;
report that rather than forcing a needless discovery. The adaptive treatment is
the shipped skill's inspect/read/stop decision policy under fixed experimental
limits, not its optional budget expansion or query reformulation. Freeze the
current generated skill as provenance, but expose the common prompt plus one
arm's treatment to each evaluated agent; distributing competing treatment prose
would contaminate the comparison. A future whole-skill comparison is separate.

The executable flags live in `SEARCH` and `FETCH`; record full argv at execution.
All discoveries use hybrid ranking, collection minimum ten, top-k five, all default
providers and a five-second search deadline. Metadata hybrid uses the same policy
with absent bodies, rather than silently switching to snippet ranking. Bundled
search fixes three candidate slots, a **two-second** fetch-stage budget (the #190
shipped default), ten-second per-page timeout, 2,000 retained characters per page
and 1 MB response cap. Direct fetch fixes ten seconds, 20,000 characters and 1 MB.
Those per-operation text differences are declared workflow mechanisms; the total
agent-visible evidence ceiling is shared. Cache is off; remote telemetry is off;
upstream caches/network state remain uncontrolled. No extra engines, pre-ranking,
score thresholds, concurrency overrides or ranking ablations are allowed.

Each task gets two searches, six attempted pages (including bundled fetches,
failed requests and refetches), ten model turns, 20,000 evidence characters, and
120 seconds end to end. Each CLI subprocess has an outer 30-second bound clamped
to remaining task time. The adapter must reserve up to three remaining page slots
before bundled search; fewer than three remaining slots disallows that operation.
Actual started fetches consume slots, and unavailable diagnostic counts invalidate
resource comparability rather than counting as zero. Unused slots are not retries.
The adapter counts the complete delivered tool response, including metadata and
repeated text. It must retain raw output locally and clip only the delivered view
before the ceiling, recording both and preventing citations to unseen suffixes.

Freeze one lexical initial query per task before outcomes are seen, with independent
review that entities, versions, dates and site constraints retain the question's
intent. The only second discovery is an unchanged-query retry. Query reformulation,
collection expansion and ranker weights are separate experiments. Provided-URL
allowlists come from the question; subsequent URLs must come from that task's
retained discoveries. Never fetch remembered answer-key URLs.

## Readiness and blinding

Before a full trial batch:

1. Run the canonical q01–q10 gate using the final tested executable and generated
   skill; retain every attempted window. A failing gate keeps this PR draft.
2. Establish the coding pilot's existing source-discoverability prerequisite,
   adjudicate alternate release answers against its publisher snapshots, and
   record per-task failures. The ten-question gate alone does not establish
   coverage of these 30 tasks. Do not treat historical snapshots as current
   discoverability or silently omit hard questions.
3. Add versioned, repository-disjoint development/held-out strata covering general
   discovery, provided URLs, detailed quotations, failures and long pages. The
   current release pilot is narrow. Music tasks remain exploratory and excluded
   from confirmatory averages. Freeze prompts, queries, rubric and budgets after
   development, before exposing held-out evidence.
4. Implement an adapter exposing only constrained search/fetch tools and the
   allowlisted single-question packet in a fresh context/container. No repository
   mount, answer-key path, model filesystem shell, evaluator judgments, snapshots,
   prior answers, or cross-task memory. Test isolation with canary secrets before
   spending on live trials. A prompt saying “do not read” is not isolation.
5. Verify model access and resolved identity, capture executable revision, tree/diff,
   SHA-256, generated-skill hash, plan/task/query hashes and environment policy.
   Validate actual commands and URLs in the adapter; the offline validator does
   not substitute for enforced tools or independently attest those identities.

Run a separate closed-book baseline once per task per round under matching model,
turn and wall limits, before retrieval outputs can enter that context. Record
baseline order separately; do not count it as a fourth retrieval treatment. The
six permutations balance position and directed arm transitions per task. Alternate
task ordering across rounds and pace CLI invocations by at least 0.25 seconds;
include pacing and repeated initialization in task latency. No concurrent live
provider calls across tasks. Six rounds give 540 retrieval trials plus 180
closed-book trials for the existing pilot, **not an authorization to bypass the
readiness gate**. Preserve interrupted and failed attempts with distinct run IDs.

## Trajectory and evaluator contract

The executable synthetic fixture in `test_workflow_eval.py::fixture` defines the
v1 import shape. Each trajectory retains task/arm/model, plan/binary/skill hashes,
UTC start/end, monotonic total duration and terminal status. Ordered model/tool
events carry monotonic offsets; model events retain complete request/response and
actual tokenizer-specific usage where supplied. Tools retain argv, pre-call reason,
exit/timeout state, stdout/stderr, actual page attempts, observed response bytes,
exact delivered response, and evidence IDs with URL, snippet/page kind, full retained
passage and SHA-256. Retain provider diagnostics and candidate artifacts in stdout
or referenced local raw run directories; never infer omitted candidates from top-k.
The final answer includes explicit abstention and claim-level evidence citations.

Validation rejects fabricated/unseen citations, evidence drift, non-finite or
nonsequential timings, missing receipts, workflow contamination, and closed-book
tool use. Over-budget and failed attempts remain visible, never dropped. Imported
provenance is a declaration, not a cryptographic attestation of tool execution.
The adapter must verify source URLs, exact permitted argv, telemetry/cache setup,
receipt clock integrity and evidence correspondence to raw output. Interrupted
attempts without complete receipts remain invalid/missing, never passing trials.

A separate judgment binds to the canonical trajectory hash and identifies the
assessor. Each dimension has its own score and rationale: 0 unsupported/incorrect,
1 partial/mixed, 2 fully meets the criterion; null is unknown:

- Correctness: every answer claim agrees with the adjudicated answer key or a
  documented valid alternate. A lucky unsupported correct answer can score here.
- Citation support: each substantive claim is entailed by its actual retained
  cited passage; exact quotation and sufficient context where requested.
- Completeness: all requested atomic facts, dates, versions and qualifications.
- Source quality: requested primary/official authority and temporal relevance.
- Appropriate abstention: admits genuinely unavailable evidence without inferring
  nonexistence; does not abstain when sufficient evidence was supplied. A complete
  warranted answer also earns two, so no forced abstention is rewarded.

Relevance judgments from #78 are separate input diagnostics, never answer scores.
Unknown dimensions stay unknown. A supported complete answer requires correctness,
support and completeness all two, a completed non-abstaining cited answer, and no
budget violation; source quality and abstention remain independently reported.
The validator cannot establish entailment by string matching. Independently review
small-model judgments on development examples and all disagreements before freezing
held-out grading. Report assessor identity and uncertainty; do not silently promote
a heuristic or a model's self-assessment into an answer key.

## Ranking reuse and reporting plan

Reuse `examples/rank_replay.rs` and `report_rank_replay.py` from #78 on the complete
frozen `candidates`/`queries` artifacts. Preserve their input hashes and separate
provider/snippet/body/hybrid/RRF replay from live workflow timings. Synthetic
fixtures here test evaluator bookkeeping; they do not replace production rank
replay or certify the existing release answer keys.

For every task/arm/round, retain failed/empty trials and report all five quality
dimensions, supported-complete rate, closed-book-failed subset, and regressions.
Report total-task p50/p95 using **nearest rank**, tool process and model-turn time
separately, search/page attempts, observed response bytes, delivered characters and
actual input/output tokens with tokenizer ID. Unknown bytes/tokens stay null, never
estimated from characters or summed across differing tokenizers. CLI diagnostics
may omit some wire bytes: label observed decoded response bytes, not network cost.
Report unaccounted task time (scheduling, initialization, pacing) rather than
attributing it to the model. Timeouts remain censored at the declared bound.

Before any recommendation, require all tasks/rounds present, no unresolved isolation
or resource-policy violations, and complete independent judgments. Use paired
per-task differences and query-cluster bootstrap intervals (10,000 resamples,
seed 84); summarize held-out separately. Six repeats are not 180 independent
tasks. Predeclare a two-percentage-point noninferiority margin for supported
complete-answer rate and no loss of the only supported answer on any task.
Recommend task-specific guidance only when the lower 95% interval exceeds that
quality margin and the upper interval for paired resource change is below zero
for the declared primary resource (total task seconds). Otherwise recommend **no
change** or more evidence. Report source-quality/abstention regressions regardless
of the primary metric. These small datasets may be unable to resolve that margin.
No default change follows solely from command latency; do not generalize synthetic
checks or one live acceptance window into workflow superiority.
