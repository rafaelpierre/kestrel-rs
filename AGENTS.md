# Repository guidelines

These instructions apply throughout this repository. Follow more specific
`AGENTS.md` files within their scope. If `RTK.md` is available, read and follow
its additional tooling guidance; if it is missing, report that fact and continue
with these instructions rather than guessing its contents.

## Project context

- `kestrel-rs` provides keyless web search, page extraction, and BM25 ranking.
  The library is `kestrelsearch`; the executable is `kestrel`.
- Use Rust edition 2024 and preserve the minimum supported Rust version declared
  in `Cargo.toml` (currently 1.89). Do not raise it incidentally.
- Read `README.md`, `Cargo.toml`, relevant files under `docs/`, and the affected
  implementation and tests before changing behavior.
- Keep library APIs, CLI flags, JSON output, provider ordering, diagnostics,
  skill locations, and benchmark artifacts compatible with documented contracts.
  Explicitly scope and document any intentional breaking change.

## Way of working

- Inspect repository status, local instructions, and the issue before editing.
  Preserve unrelated changes and never discard another contributor's work.
- State the intended outcome, acceptance criteria, and validation approach.
  For substantial work, outline a short plan and update it when scope changes.
- Implement the smallest cohesive change that meets the issue's acceptance
  criteria. Avoid unrelated refactors, dependency upgrades, and formatting churn.
- Prefer existing patterns and dependencies. Explain why a new dependency or
  abstraction is needed, including its maintenance and compatibility cost.
- Keep the user informed about material findings, scope changes, and blockers.
  Finish with what changed, validation results, and outstanding limitations.
- Never claim a command passed, a review was resolved, or a remote operation
  succeeded without checking the result. If access or credentials are missing,
  report the exact blocker and continue any independent work that is possible.

## Local Honeycomb configuration

- Before asking for Honeycomb credentials, check the local `.env` configuration.
  The existing telemetry configuration is in
  `/Users/rafaelpierre/projects/kestrel-rs-issue-146/.env`; isolated worktrees do
  not automatically inherit it. Also check the active worktree and primary
  checkout for a local `.env` if the configuration has been moved.
- Use `HONEYCOMB_API_KEY` from that file for authorized OTLP ingestion. Load only
  the needed variables into the diagnostic subprocess; keep secrets out of
  command output, traces, reports, commits and PRs. Never commit `.env`.
- Honeycomb MCP access does not configure CLI trace export. Confirm the endpoint
  and environment, enable export for the requested run, and verify the run ID
  through Honeycomb MCP before claiming traces were ingested.

## Scope work through GitHub issues

- Before implementing a feature, search the GitHub backlog, including closed
  issues and related PRs. Reuse an existing issue when it covers the work.
- Always create a GitHub issue when the feature is not already in the backlog.
  Link all relevant existing issues and explain overlaps or dependencies. Do not
  create duplicates or silently reopen work that was intentionally declined.
- Give issues a concrete problem statement, desired behavior, acceptance
  criteria, scope boundaries, and a proposed validation approach. Include
  reproduction steps and expected versus actual behavior for bugs.
- Always label issues for type, topic/area, and priority or impact. Inspect and
  reuse the repository's label taxonomy; create a clearly named missing label
  when necessary. Explain priority or impact in the issue instead of assigning
  an arbitrary severity.
- Split independent features into linked issues. Record meaningful scope changes
  and newly discovered follow-up work in GitHub rather than burying them in a PR.
- For issues of type `investigation`, create an investigation summary report
  and save it in the appropriate Notion folder for the project or topic. Give
  the report a descriptive title, such as `Investigation #<issue>: <subject>`,
  summarize the findings, supporting evidence, conclusions, and next steps,
  and link the report from the GitHub issue.

## Feature branches and isolated worktrees

- Whenever starting implementation of an issue, always use a dedicated feature
  branch in a separate Git worktree. Do not implement directly on `main` or in
  the shared primary checkout.
- Fetch the remote and create the worktree from the current target branch,
  normally `origin/main`. Use `codex/<issue-number>-<short-description>` unless
  the user specifies another branch name. Put worktrees outside this checkout.
- If the issue already has an active branch and PR, inspect and reuse its
  isolated worktree instead of creating competing implementation branches.
- Keep edits, builds, and tests in the issue's worktree. Confirm the working
  directory and branch before committing or pushing.
- After the PR is confirmed merged, delete its worktree. First check for
  uncommitted or untracked work and preserve anything valuable; never force
  removal to hide a dirty state. Remove the associated local feature branch
  once its work is accounted for in the merge, including squash merges.

## Rust implementation practices

- Keep responsibilities in the existing modules: CLI parsing in `src/cli.rs`,
  client APIs in `src/client.rs`, search orchestration in `src/search.rs`,
  provider handling in `src/providers.rs`, fetching in `src/fetcher.rs`, and
  ranking in `src/ranking.rs`. Follow adjacent code for shared transport,
  configuration, models, caching, and diagnostics.
- Prefer explicit types, clear ownership, borrowed inputs where practical, and
  small functions with focused responsibilities. Avoid unnecessary cloning and
  speculative generic abstractions.
- Propagate recoverable failures with `Result` and the existing typed errors.
  Avoid `unwrap`, `expect`, and panics in production paths unless an invariant
  makes failure impossible and that invariant is documented.
- Keep blocking I/O and expensive parsing off async executor threads. Preserve
  bounded concurrency, timeouts, cancellation, retry limits, and response-size
  limits. Reuse shared clients and connection pools.
- Treat fetched pages and provider responses as untrusted input. Handle malformed
  markup, encoding errors, redirects, empty results, and partial provider failure
  without crashing or weakening TLS verification.
- Preserve canonical-URL deduplication, provenance, cache-key semantics, and
  deterministic ordering where promised. Make performance/coverage tradeoffs
  explicit rather than silently changing defaults.
- Keep machine-readable results on stdout and diagnostics on stderr. Never log
  credentials or commit secrets, private configuration, or sensitive captures.
- Document public APIs and non-obvious invariants. Avoid introducing `unsafe`;
  if unavoidable, document the safety contract and validate it explicitly.

## Validation and documentation

- Add focused regression tests for bugs and meaningful tests for new behavior.
  Cover relevant failure paths and boundaries, not just the happy path.
- Use local mock servers and sanitized fixtures for network-facing tests.
  Keep the ordinary test suite deterministic and independent of live providers.
  Keep shared environment or filesystem state isolated between tests.
- Run targeted tests during development. Before handing off Rust changes, run
  the repository's CI checks from the feature worktree:

  ```sh
  cargo fmt --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  ```

- Run `cargo build --release` when changes affect the executable, dependencies,
  packaging, or release behavior. Check the declared minimum Rust version when
  using new language/library features or changing dependencies.
- For documentation-only changes, check accuracy, links, and the diff; a full
  Rust build is not normally required. This does not waive the mandatory
  ten-question gate below or its executable-provenance requirements. Report any
  skipped or blocked checks and why.
- Use the methodology in `benchmarks/README.md` for performance claims. Record
  configuration, sample size, and limitations; distinguish live-network variance
  from reproducible improvements. Do not commit generated artifacts by default.
- Update README examples and relevant `docs/` when behavior or usage changes.
  Preserve the release-plz/cargo-dist release workflow; do not manually bump
  versions or rewrite generated release files unless that is the task's scope.

## Mandatory ten-question evidence-quality gate

- **Every feature and every change must test all ten questions, q01–q10, in
  [the canonical dataset](benchmarks/codex-search-2026-09-11/queries.json).
  This includes fixes, refactors, dependencies, packaging, configuration, tests,
  skills, documentation and repository instructions.** Run the gate before
  marking a PR ready or merging; it supplements, never replaces, the Rust checks
  and generated-skill compatibility checks above/below. It is a live acceptance
  exercise, separate from the deterministic ordinary test suite.
- **The minimum is 10/10 individual passes.** Each question needs at least one
  directly relevant source and enough retrieved evidence to write an answer
  meeting its row below. Inspect all returned titles, URLs and snippets; select
  sources deliberately and fetch supporting passages when snippets cannot
  establish the answer. A relevant-looking URL, exit zero, nonempty output,
  successful extraction, keyword overlap or an aggregate average is not a pass.
  Partial answers, wrong entities/versions, generic homepages, unsupported claims,
  empty results, abstentions and missing/unrun/unknown judgments do not pass.

| ID | Minimum acceptable evidence and answer |
| --- | --- |
| q01 | Explicitly identify **Canberra** as Australia's capital, supported by retrieved text; a city URL alone is insufficient. |
| q02 | Explain blue sky through **Rayleigh scattering**, including preferential scattering of shorter visible wavelengths; dictionary definitions or generic atmospheric context fail. |
| q03 | Use official Python documentation to explain **TaskGroup** failure propagation, sibling cancellation, grouped exceptions and `except*`, distinguishing ordinary failures from cancellation; preserve readable code when used. |
| q04 | Retrieve **postgresql.org** documentation and explain what **EXPLAIN ANALYZE** executes/reports and what **BUFFERS** adds; a generic PostgreSQL page fails. |
| q05 | Retrieve **grafana.com** documentation showing **TraceQL parent/child structural relationships**, with a supported operator or query example and correct direction; distinguish immediate children from descendants. |
| q06 | Retrieve the requested official **learn.chatgpt.com** Codex configuration evidence for **otel / trace_exporter**, including supported configuration syntax; a CLI landing page fails. A documented official redirect is acceptable; an unannounced domain substitution is not. |
| q07 | Use official Rust documentation to explain **E0382/use of moved value**, with at least one applicable repair and its ownership implications; game pages or a Rust homepage fail. |
| q08 | Retrieve dated evidence addressing **JWST observations of TRAPPIST-1 b in the requested 2025 context**, accurately stating the atmospheric finding and its uncertainty; evidence about planet e or another year alone fails. |
| q09 | Retrieve official **Grafana Tempo 3.0** release/migration documentation and identify its documented breaking changes with actionable migration implications; another release or the Grafana homepage fails. |
| q10 | State the **London Science Museum's opening hours** from retrieved evidence, preferably its official visitor page, preserving stated date/season/holiday qualifications; London landmarks or another museum fail. |

### Required execution and evidence

- Use only keyword-based full-text search (FTS) queries. NEVER send conversational
  questions or semantic prompts to `search`; translate them to lexical terms while
  preserving intent, entities, identifiers, phrases, negation, domains, versions
  and dates. This applies to initial and recovery queries and does not introduce
  a local Boolean parser. See the [dataset revision notes](benchmarks/codex-search-2026-09-11/README.md).
- Start each question with its exact manifest query and retain its ID/intent.
  Use Kestrel's tested executable and the current generated skill to discover,
  inspect and selectively read. Do not fill gaps from model memory, another
  search tool, remembered source URLs or a copied answer key. Fetch URLs selected
  from this run; follow documented links/redirects only with provenance recorded.
- Declare the workflow, flags, budgets, cache state, model/assessor and grading
  criteria before running. Use a consistent bounded policy: at most two discovery
  calls and three direct fetches per question, with explicit finite search and
  fetch timeouts. Record any recovery query and why it is needed; preserve the
  original intent, site restrictions, entity, version and date. Do not silently
  switch syntax, loosen constraints or tune the policy after seeing failures.
  A changed policy requires a separately identified complete ten-question run;
  retain every earlier attempt rather than cherry-picking successful rows.
- Exercise the PR's final executable inputs, recording the tested revision,
  tracked diff/tree identity, binary path/version/SHA-256 and dataset hash. Build
  in the feature worktree when needed to establish this provenance; an installed
  version string alone is insufficient. Re-run after executable, dataset or
  workflow/skill changes. For documentation-only edits after a recorded run,
  reuse is allowed only with verified unchanged executable inputs, dataset and
  evaluation policy; identify both revisions and the reason in the PR.
- Save full argv, timestamps, exit codes, stdout/stderr, all returned candidates,
  provider diagnostics, actual fetch attempts and complete retained page text.
  For each question record the synthesized answer, selected URLs, supporting
  passages, pass/fail judgment and rationale; a truncated page prefix is not an
  answer. Record successful extraction separately from useful evidence and
  distinguish skipped fetches from failures. Keep sensitive/raw generated data
  local by default; publish a sanitized per-question evidence summary sufficient
  for review and state where the full artifacts are retained.
- Report discovery and fetch timings separately, plus total tool wall time per
  question including recovery; distinguish these from agent end-to-end latency.
  Use a declared percentile convention. Do not mix fetched and metadata-only
  searches into an unlabeled latency comparison. A single live run is an
  acceptance observation, not a statistically reliable performance claim.

### Readiness and failure handling

- Every PR description must link the dataset and a ten-row q01–q10 results table,
  with evidence references, individual judgments, timing, tested revision/binary
  identity, exact workflow and limitations. State **gate PASS (10/10)** only when
  all ten rows meet their minima. Missing evidence means **gate NOT PASSED**.
- Provider blocks, rate limits, deadlines, unavailable/moved documentation and
  pre-existing baseline failures must be reported honestly and keep the gate
  unsatisfied. A failing baseline is not a waiver, and improvements elsewhere
  cannot compensate for a failed question. Preserve failed runs; link the
  applicable investigation/fix issue rather than weakening the dataset or rubric.
- If the gate cannot pass, push the signed work as a **draft PR**, list each
  failing/blocked question and its reason, and continue independent validation.
  Do not claim readiness or merge. Updating an obsolete query or changing this
  acceptance policy requires an explicit, separately reviewed change documenting
  the preserved intent; never silently replace a difficult question in a run.

## Required installed-skill compatibility

- **Every change to CLI parameters, arguments, or contracts, and every change to
  search or fetch behavior, must be reflected in the generated `SKILL.md` in the
  same PR. This is part of implementation completeness, not optional follow-up
  documentation.** The skill installed by `kestrel skill install` must accurately
  describe the installed version of Kestrel.
- The skill template and generator live in `src/skill.rs`
  (`generate_skill_md`); option tables are derived from the Clap command tree in
  `src/cli.rs`. Update CLI help metadata and the template's prose, schemas,
  examples, and usage guidance together as applicable. Automatically generated
  option tables do not replace documenting behavioral changes.
- Cover added, renamed, removed, or deprecated arguments; defaults and accepted
  values; argument interactions; output schemas and exit/error behavior; and
  search/fetch semantics such as providers, ranking, timeouts, budgets, limits,
  caching, and partial failures. Remove stale guidance and explain migrations
  for breaking changes.
- Verify the generated skill and install it into a temporary project using the
  updated binary. Check the resulting `SKILL.md` against current CLI help and
  behavior, and validate affected examples with deterministic fixtures or local
  mock servers where possible. Add or update focused generation/installation
  regression tests for changed contracts. Do not overwrite a user's installed
  skill merely to test generation.
- Include the skill updates and compatibility validation in the PR description.
  Changes are not ready to merge while the installed skill teaches an obsolete
  CLI contract or search/fetch behavior.

## Signed commits and pull requests

- All commits must have verified signatures. Check that signing is configured
  before committing, sign every commit (for example, `git commit -S`), and verify
  locally. After pushing, confirm GitHub reports the commits as **Verified**.
  A sign-off trailer is not a cryptographic signature. Never bypass signing;
  check available signing alternatives before reporting a blocker.
- GitHub supports automatic commit signing through its API using the existing
  authenticated login. Use the
  [createCommitOnBranch mutation](https://docs.github.com/en/graphql/reference/commits#createcommitonbranch)
  as an alternative when local signing is unavailable (for example, an expired
  local key). GitHub signs these commits when supported; do not assume an API
  success alone proves signature verification.
- Verify the resulting commit signature locally and confirm GitHub reports it as
  **Verified**. Confirm its complete Git tree, including file modes, exactly
  matches the tested worktree and its parent is the intended base before updating
  the PR branch. For a rebase, create the signed commit on a temporary branch at
  the intended base, verify it, then update the PR branch with an explicit
  `--force-with-lease` against the previously observed head. Remove the temporary
  branch after confirming the PR points to the verified commit. If neither local
  signing nor GitHub API signing yields a verified commit, report the blocker;
  never publish an unsigned substitute.
- Use Conventional Commits: `fix:` for fixes, `feat:` for features, `docs:` for
  documentation, and an appropriate other type for maintenance. Mark intentional
  breaking changes with `!` or a `BREAKING CHANGE:` footer; release-plz uses these
  messages to determine versions. Ensure any squash title follows the same rule.
- Review the diff for accidental files, secrets, and scope creep before commit.
- When feature work is done, push the signed branch to the remote and create a
  PR, or update the existing PR. Do not stop at local commits.
- Describe the problem, resulting behavior, key implementation choices,
  validation performed, compatibility implications, and remaining limitations.
  Include `Closes #<issue>` only for issues fully addressed; link other related
  issues without closing them. Keep the title and description aligned with the
  final scope. Use a draft PR if work or required validation remains incomplete.

## Existing PR reviews and merging

- Before pushing updates to an existing PR, inspect prior reviews and all
  unresolved review threads, especially those from Codex. Check outdated threads
  as well as current ones; an outdated diff does not mean feedback is resolved.
- Address applicable Codex feedback within scope. After the fix is pushed and
  validated, reply with the fix and evidence, then resolve the corresponding
  review thread. Do not resolve comments merely because new commits exist.
- Explicitly raise every remaining unresolved Codex comment in the PR update and
  final handoff, with a link, its impact, and the reason it remains open. If a
  comment is disputed or deferred, explain why and link a follow-up issue where
  appropriate; do not silently dismiss it or mark it fixed.
- After pushing, check CI, signature verification, and any new review feedback.
  Report pending checks as pending rather than claiming the PR is ready.
- Merge only when authorized, the ten-question gate passes, required checks and
  approvals pass, and unresolved feedback has been addressed or explicitly
  accepted by a maintainer. Preserve
  verified signatures in the resulting history; never bypass branch protection.
- Confirm the remote PR is merged before cleaning up the worktree and branch.
  Report the PR link, issue status, validation, and cleanup outcome.

### Merging is blocked

- **All commits in the PR must have verified signatures.** Confirm GitHub shows
  **Verified** for every commit on the latest pushed revision before merging.
  Any unsigned, unverified, or unverifiable commit blocks the merge, even when
  CI and approvals pass. Resolve signature verification and recheck the updated
  PR before proceeding; never bypass this requirement.
