# Advisory content quality (issue #77)

A successful extraction can contain no useful evidence. Version 1 adds a bounded,
explainable assessment of **retained text**, without treating short text as bad,
certifying factual relevance, rejecting pages, or changing ranking weights.

## States, reasons, and ownership

`content_quality::assess_content_quality(Option<&str>)` owns this signal. It is
independent of `FetchOutcome`, download completeness, byte/character limits,
provider outcomes and query relevance. `ContentQuality` serializes as
`{"version":1,"state":"boilerplate_only","reasons":["browser_error_messages"]}`.

| State | Meaning | Reasons |
| --- | --- | --- |
| `boilerplate_only` | Entire normalized retained text is recognized shell messages, with a specific browser message or at least two distinct messages | `browser_error_messages`, `comment_messages` |
| `unflagged` | No known shell message detected; **not** a useful-evidence verdict | `no_known_shell` |
| `unknown` | Missing/empty text, input too large, mixed text, or insufficient signals | `no_text`, `assessment_limit_exceeded`, `mixed_content`, or a message reason plus `insufficient_signals` |

Usefulness cannot safely be inferred from the absence of known error messages.
There is deliberately no `useful` state. Callers must inspect evidence and context.
A shell-only retained prefix may belong to a useful full page. A missing body
might reflect failure, cancellation, or extraction loss; `no_text` does not choose
between those explanations. Consult the existing request report separately.

Assessment accepts at most 32,768 UTF-8 bytes. Larger inputs return unknown without
judging a prefix. Within that bound, normalize whitespace and ASCII case only;
keep quotation marks, punctuation, code and non-ASCII characters. Matching consumes
whole messages separated by whitespace. Any unmatched material prevents a shell
verdict. Mixed-message detection also checks whitespace boundaries. Work is bounded
by this input cap and a fixed 13-entry vocabulary; no regex backtracking, network
calls, semantic models or new dependencies are involved.

The vocabulary is intentionally small and English-only:

- Specific browser messages: `Your browser is not supported` and
  `Please update your browser`, each with or without a final period.
- Generic messages: `Something went wrong` and `Please try again later`, each
  with or without a final period. One alone is unknown.
- Comment messages: `Leave a Reply`, `Post a comment`,
  `Your email address will not be published.`, `Required fields are marked *`,
  and `You can use some HTML tags, such as <b>, <i>, <a>.`.

Repeated headings and final-period variants count as one distinct signal, not
corroboration. Generic/comment messages need a second distinct message. There are
no hostname, article-title, arbitrary navigation or broad error-substring rules.
An unquoted answer consisting exactly of recognized shell messages is inherently
ambiguous and can be a false positive. This is one reason the signal stays advisory.

## HTML root recovery

After existing chrome removal and root selection, assess the selected extraction.
If it is shell-only, inspect **at most the first explicit `article`** in the same
already-downloaded DOM. Use its bounded extraction only if it is nonempty and
unflagged or mixed. Do not try additional articles, change selection for ordinary
or unknown bodies, accept an over-assessment-limit alternative, or discard a shell
when recovery fails. Parsing and this fallback run on the existing bounded blocking
executor. No extra HTTP request is made.

This recovers an explicit article hidden by a comments-only `main`. It is not a
universal readability fix: an unrelated first article could be selected, missing
JavaScript content cannot be recovered, and content removed by existing structural
chrome rules cannot be restored. #20 supplies whole-token chrome matching; #22
adds [ordered text, short answers and code](page-extraction.md#ordered-readable-text-issue-22).
The original HTML may still contain useful text outside the selected root.

## Library, CLI, ranking and cache compatibility

- `FetchReport::content_quality(index)` assesses `contents[index]`, preserving
  original input indexing. Invalid indices return `None`; missing bodies return
  an unknown assessment. It does not join against `pages`, which may be reordered
  or incomplete after caching/cancellation.
- `SearchResult::content_quality()` assesses the body, removing only the exact
  `Source: <this result's URL>\n\n` wrapper used by CLI fetching. The standalone
  assessment function expects body text without a provenance wrapper.
- Existing public struct fields and result schemas are unchanged. Default CLI
  diagnostics now expose advisory quality; `--no-diagnostics` restores the previous
  envelopes. See [structured diagnostics](structured-diagnostics.md).
  Standalone fetch still returns a flagged body, URL and elapsed time successfully;
  text output and exit codes remain compatible. Quality itself has no control flag.
- Search retains content, metadata and provenance. Body/hybrid BM25 continue to
  consume flagged bodies, and other ranking policies remain unchanged. Rejection
  and ranking penalties require broader measured evidence; #78 owns ranking effects.
- Opt-in search artifacts add `results[].content_quality` and
  `diagnostics.candidate_content_quality` (aligned with `candidates`, including
  missing bodies). Existing artifact fields remain intact. Standalone fetch
  does not emit search artifacts. Default command diagnostics are documented separately.
- Assessments are computed on demand, never written into the page cache.
  Cache keys, TTL, stored text and byte-cap exclusion remain compatible. No
  quality-version cache migration is needed: a cache hit gets the current
  assessment of its stored text. Previously cached extractions do **not** gain
  root recovery until expiry or a fresh fetch. The assessment version identifies
  the policy used in saved artifacts; future vocabulary/policy changes must bump it.

## Frozen evaluation and extraction evidence

The JSON fixtures in [tests/fixtures/content-quality](../tests/fixtures/content-quality)
contain source HTML or literal text, exact expected extraction where applicable,
a usefulness judgment, expected state and an explanation of the source of loss.
They are **authored synthetic structures**, not captured September 12 site HTML.
They reproduce the reported classes without claiming to reproduce Blogspot,
SoundCloud or Stringjoy's current production markup. No live-site accuracy or
latency claim is made. Do not introduce site-specific rules without captured,
sanitized original HTML.

Development fixtures were used to define the policy. The separate held-out set
was authored after the vocabulary was written and evaluated without tuning that
vocabulary. Both sets have the same author; they are not an independent benchmark.
Issue #22 updates exact extraction expectations for source order and line breaks.
The HTML cases retain `original_extracted` and `original_state` to preserve the
original observations. The two formerly empty short-answer/code HTML cases now
retain `42` and `let answer = 42;` and assess as unflagged. Vocabulary, usefulness
judgments and the original HTML/text inputs are unchanged.

At a 20,000-character extraction limit, default chrome/root rules, no cache
and no network, the deterministic corpus counts remain:

| Set | Cases | Shell true positives | Shell false positives | Missed unusable cases | Useful cases not flagged |
| --- | ---: | ---: | ---: | ---: | ---: |
| Development | 6 | 2 | 0 | 1 | 3 |
| Held-out | 18 | 3 | 0 | 4 | 11 |

Unknown and unflagged count as **not flagged** in this table. The two formerly
empty HTML cases now retain evidence, but their change from unknown to unflagged
does not change these counts. The table measures the shell flag, not complete
extraction accuracy. There are no observed false-positive shell flags in these 24 fixtures;
that does not establish a population false-positive rate.

The five missed unusable cases are title/navigation/reply mixtures, a lone generic
failure, a repeated generic failure, an unrecognized French browser error and
unrecognized navigation. These remain deliberate limitations, not silently
relabelled successes. Explicit recovery restores the two fixtures whose old
main-first selection returned only comment instructions. Tests assert both the
old lost-body extraction and complete recovered body. Useful article/comment
mixtures retain the article, and browser-error documentation stays unflagged or
unknown. Incidental chrome names use the existing #20 fix.

Reproduce the judgments and exact extractions with:

```sh
cargo test quality -- --nocapture
```

The local mock-server tests exercise direct text/JSON fetch, search attachment and
body/hybrid ranking, cache hits, failures, source wrappers and schema compatibility.
The CLI suite installs the generated skill into a temporary project/home and
compares its embedded option tables against actual CLI help. Full validation uses
`cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features` and `cargo build --release`. This follows the
[benchmark methodology](../benchmarks/README.md) distinction between reproducible
fixture evidence and live-network/answer-quality measurements.
