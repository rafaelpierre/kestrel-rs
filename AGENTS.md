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
  Rust build is not required. Report any skipped or blocked checks and why.
- Use the methodology in `benchmarks/README.md` for performance claims. Record
  configuration, sample size, and limitations; distinguish live-network variance
  from reproducible improvements. Do not commit generated artifacts by default.
- Update README examples and relevant `docs/` when behavior or usage changes.
  Preserve the release-plz/cargo-dist release workflow; do not manually bump
  versions or rewrite generated release files unless that is the task's scope.

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
  report a blocker if a usable signing key or GitHub verification is unavailable.
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
- Merge only when authorized, required checks and approvals pass, and unresolved
  feedback has been addressed or explicitly accepted by a maintainer. Preserve
  verified signatures in the resulting history; never bypass branch protection.
- Confirm the remote PR is merged before cleaning up the worktree and branch.
  Report the PR link, issue status, validation, and cleanup outcome.

### Merging is blocked

- **All commits in the PR must have verified signatures.** Confirm GitHub shows
  **Verified** for every commit on the latest pushed revision before merging.
  Any unsigned, unverified, or unverifiable commit blocks the merge, even when
  CI and approvals pass. Resolve signature verification and recheck the updated
  PR before proceeding; never bypass this requirement.
