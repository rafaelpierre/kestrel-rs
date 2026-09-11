# Fresh coding pilot: release evidence, September 11, 2026

30 authored questions: 10 development and 20 held out, with repository-disjoint
splits. Sources are publisher GitHub releases dated June 13–September 11, 2026.
`questions.json` contains no expected release tags or behavior answers.
`answer-key.json` contains release tags, atomic change facts, gold URLs, publication
dates and snapshot hashes. `sources/` preserves primary release snapshots.
`../build_coding_pilot.py` regenerates the authored template questions from these
snapshots. It never downloads or silently refreshes source material.

This is a narrow release-identification and behavior-change pilot, not an official
FreshStack score or a completed coding-agent benchmark. Some questions give the
PR URL; direct fetching is a legitimate evidence-gathering action. Questions
without PR links identify an API or dependency instead. The publication date
identifies the intended release without asserting that a fix was never backported.
The next dataset iteration should include migration code and multi-package
compatibility cases after these retrieval checks pass.

Do not run an evaluation from this repository with unrestricted filesystem tools:
that exposes answer keys. Supply only one question in a fresh isolated context,
with no access to this directory, for each condition and round. Keep model version,
prompt, sampling, turns, search/fetch budget and final format identical.
Conditions are closed-book, native search and Kestrel. Run three rounds, rotating
condition order. Use 5 search calls, 10 fetched pages, 10 turns, and a 20,000-character
evidence budget per question as the pilot defaults. Record interface differences.

A run must save final answers, the complete tool trajectory, timestamps and the
actual retrieved passages. Report release/behavior correctness separately from
citation support and completeness. A correct answer without supporting retrieved
evidence fails grounding. Report all questions and the closed-book-failed subset;
never silently remove questions that the model already knows.

Before scoring: verify source discoverability through both backends and adjudicate
any alternate release answers against primary sources. Release membership and
change descriptions have been checked against saved publisher notes; executable
fixes, model novelty, independent review, and live index coverage are not yet
validated. No agent trials or score claims are included in this directory.
