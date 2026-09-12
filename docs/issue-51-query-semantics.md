# Issue #51: portable query constraints

Issue: https://github.com/rafaelpierre/kestrel-rs/issues/51

Date: 2026-09-12. Base: `0977b88`.
Worktree: `kestrel-rs-issue-51`; branch: `codex/issue-51-query-semantics`.

## Implemented behavior

All nine providers use a shared portable query parser and result filter by
default. It supports exact quoted phrases, implicit/explicit AND, OR, NOT,
exclusions, parentheses and Boolean hostname restrictions. The original query
is sent intact as a provider retrieval hint; local checks enforce the lexical
contract without assuming upstream operator support. Unsupported syntax fails
before requests; `--query-syntax native` preserves previous passthrough behavior.
The Rust API exposes `QuerySyntax` and `SearchOptions.query_syntax`.

Checks use title/snippet evidence before quorum, deduplication, fetching or
ranking. Rejected results cannot cancel other providers through quorum. Raw
counts, original ranks and `filtered_empty` diagnostics survive filtering. CLI
stderr reports the number excluded. Site restrictions use the expression tree in
portable mode and the existing conservative hostname extraction in native mode.
The shared HTTP(S) URL validity check applies in both modes.

See `search-providers.md` for grammar, precedence, escaping, size limits and API
migration. This is metadata matching, not proof of semantic relevance or complete
page contents. Required terms missing from metadata exclude a result, possibly
losing a relevant page; NOT checks absence only in metadata. A provider may still
fail to retrieve matching pages. Issue #32's upstream diagnosis remains open.

## Reproduction before implementation

A twelve-case sweep of installed `kestrel 2.0.0` reproduced unrelated Bing results
for quoted, plain and AND inputs, including default fanout. Its precise source
revision was not established. Swisscows returned on-topic results. Yep was empty;
other providers timed out, returned HTTP errors or a bot challenge. Source
inspection and request-serialization regressions found no dropped query words.
The previous implementation only enforced conservative site restrictions locally.

## Verification

- `cargo test --all-features`: 125 passing tests, two pre-existing ignored live tests.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo fmt --check`, `git diff --check`, Python runner syntax compilation: passed.
- Deterministic coverage: CLI argument and repeated-query preservation; malformed
  input; all nine provider request encodings; phrase vs conjunction; Boolean
  precedence/exclusions/sites; Unicode and code identifiers; missing metadata;
  original ranks/provenance; native behavior; filtered diagnostics; and rejected
  results not satisfying quorum. No live provider success is required by CI.

48 live source-built CLI invocations were completed:

1. 27 retrieval-only cases: nine providers times phrase, conjunction and grouped
   phrase/exclusion inputs. Swisscows returned matching results for all three;
   Bing's unrelated results were removed. Yep was empty and other providers
   errored. These runs used a frozen build before the final separation of legacy
   native site extraction from the portable expression filter.
2. 18 final-build phrase cases: nine providers times default fetching/ranking and
   `--no-rank`. Swisscows returned matching results in both. Yep returned matching
   results in the ranked run but was empty in the no-rank run; timing/provider
   variability prevents attributing that difference to ranking. Bing was filtered
   empty in both, and other providers errored.
3. Three final-build smoke cases: default fanout removed ten unrelated Bing
   results and returned empty; native Bing returned the previous results;
   all-nine-provider fanout with quorum one returned matching results and cancelled
   four stragglers. The deterministic quorum test separately proves that an earlier
   rejected response does not cancel a later matching provider.

Provider failures/empty runs do not establish semantic support or global
availability. These bounded local sweeps do not establish upstream root cause.
No default engine set, transport or retry budget was changed.

## Reproducing the live matrix

```sh
cargo build
python3 benchmarks/query_semantics.py --binary target/debug/kestrel \
  --output benchmarks/results/query-semantics-new.json
```

The opt-in runner covers all nine providers, three query forms and three stages
by default. Select `--stages no-fetch` for retrieval only or `--query` to narrow
inputs. It records executable hashes, completion, errors, empty results and
returned metadata; it does not assign relevance judgments. Existing output files
are rejected. Incomplete runs remain marked incomplete.

Local ignored evidence: `benchmarks/results/issue-51-initial-live.json`,
`issue-51-portable-matrix.json`, `issue-51-fetch-matrix.json` and
`issue-51-final-smoke.json`. No raw provider traces are committed.

The referenced RTK.md was unavailable in the checkout, checked parents and local
Codex directory; no RTK executable was available.
