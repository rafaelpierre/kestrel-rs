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

Each matrix uses six declared query intents, Chrome 146/macOS/en-GB headers,
`gb-en` region (except the no-region control), a three-second search budget and
reused clients without a cookie jar. It alternates variant order across queries.
The variants are standard Bing transport, browser-impersonated transport, omitted
region, the observed browser `form=QBRE` parameter, default fallback, fanout quorum
1 and full fanout. Isolated transport comparisons use one request attempt;
orchestration retains production retries and site restrictions. Thus isolated
site-query raw results must not be mistaken for application results.

Use `--encodings` for a separate two-variant experiment comparing `+` and `%20`
space encoding while preserving the decoded query. Optional `--raw` retains local
unredacted HTML in the ignored output directory. It is disabled by default.
Sanitized JSON retains public query/result text, response hashes, HTTP status and
version, selected cache headers and elapsed time. Cookies and authorization are
not captured. Result URL query strings/fragments are removed; this can collapse
semantically distinct URLs, so judgments also use the full title and snippet.
Final URLs retain only q/cc. Redirect chains and browser network equivalence are
explicitly unavailable, not inferred from final URLs. The harness does not bypass
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
Only returned provider diagnostics contribute observed attempt counts; missing
error diagnostics are counted explicitly. Do not infer zero requests from absent
diagnostics. Inspect schedule.json before scoring: a failed window means the
whole requested schedule is incomplete, even if an earlier window finished.

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

See `docs/issue-32-bing-fidelity.md` for results, limitations and the decision to
keep defaults unchanged.
