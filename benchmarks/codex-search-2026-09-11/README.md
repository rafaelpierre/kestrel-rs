# Canonical evidence queries

The array in `queries.json` retains q01–q10 and each original answer intent.
Search inputs must be keyword-based full-text search (FTS) queries, never
conversational questions or semantic prompts. The generated skill requires agents
to formulate these lexical queries before invoking Kestrel. This does not add
runtime query rewriting or a provider-independent Boolean language.

## FTS correction — 2026-09-13, issue #153

| ID | Previous query | Corrected keyword query |
| --- | --- | --- |
| q01 | What is the capital of Australia? | Australia capital |
| q02 | why is the sky blue Rayleigh scattering | Rayleigh scattering blue sky |
| q07 | Rust E0382 use of moved value how to fix | Rust E0382 moved value fix |

The other seven inputs already use keyword phrases and remain unchanged. Domain
restrictions, entities, technical identifiers, versions and dates are preserved.
The full answer minima in `AGENTS.md` remain unchanged, including official-source
requirements and uncertainty. A shorter query does not make partial evidence pass.

Previous manifests and scores remain available in Git history (pre-correction
revision `339dabc`). Their natural-language inputs must not be silently relabeled
as FTS tests. Record the dataset hash for every new run; do not attribute a score
difference across this correction to a runtime improvement.

Validation uses three complete q01–q10 rounds with the same declared flags and
budgets. Keep all attempts, inspect candidates, and fetch supporting evidence.
Report per-round and combined coverage explicitly; if a fetched page is reused
across rounds, it only supports rounds that actually discovered that URL. The
live evidence exercise stays separate from deterministic tests.
