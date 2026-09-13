# HTML page extraction

Direct `fetch` and search candidate fetching share the bounded HTML extractor
for HTML/XHTML responses. `text/plain` bypasses DOM cleanup and preserves literal
text; see [plain-text decoding and limits](cli-arguments.md#plain-text-responses).
Before selecting and extracting body text, it removes `head`, `template`, `script`, `style`, `nav`,
`header`, `footer`, `aside`, and `form` elements.

For `div` and `section`, class attributes are matched as whole whitespace-separated
tokens, with ASCII case-insensitive comparison. IDs match whole names. Recognized
names are `menu`, `sidebar`, `navbar`, `topbar`, `advertisement`, `ad`, `cookie`,
`modal`, `popup`, `banner`, `nav`, and `breadcrumb`. Explicit compound names are
`ad-slot`, `ad-container`, `ad-banner`, `cookie-banner`, `cookie-consent`,
`sidebar-left`, and `sidebar-right`. IDs may also append a hyphen or underscore
and one or more ASCII digits to one of these names (for example `ad-123` or
`sidebar_2`). Numeric suffix matching applies only to IDs.

Arbitrary substrings and arbitrary compound-name fragments are not matched:
`download`, `reader`, `shadow`, `thread`, `ad-supported`, and `navigation-guide`
are not clutter markers. A matching class or ID still removes the container and
its descendants. This is a conservative naming heuristic, not general semantic
classification: an unrecognized compound name can retain boilerplate, and a
content container using an exact clutter name can still be removed. Ordered text extraction uses the rules below. See the bounded
[root-selection recovery and advisory quality policy](content-quality.md) added
for #77. Output schemas, cache keys and CLI options are unchanged.

## Regression evidence for issue #20

The isolated `download` reproduction failed before the fix with `None` instead
of useful text. The book-layout fixture also failed before the fix, retaining
only its heading and losing its body paragraph.

Three authored documentation-layout fixtures in
[`tests/fixtures/extraction`](../tests/fixtures/extraction) cover a book chapter,
an API reference and installation instructions. At a 20,000-character limit,
all three fixed extractions match their complete expected text exactly: every
expected heading/body/list item is retained and no fixture chrome text remains.
Additional table-driven tests cover class versus ID matching, nested main
content, mixed classes/case, explicit ads/sidebar/navigation, and numeric ID
boundaries. The CLI mock-server test puts its article inside `class="download"`
and verifies both JSON and text fetch output.

Reproduce with `cargo test --lib fetcher::tests` and
`cargo test --test cli`. The CLI suite also installs the generated skill into a
temporary project/home and checks the extraction guidance and live CLI help.
These are deterministic fixture results, not a live-web accuracy estimate or a
latency benchmark. General readability improvements remain outside this fix.

## Ordered readable text (issue #22)

After chrome removal and the existing root-selection/recovery policy, a single
iterative DOM walk emits text once in document order. It includes the root's own
text, all heading levels, short paragraphs, inline code, nested list items,
definition lists, block quotes, preformatted code, and table captions/cells.
Nested wrappers do not duplicate their descendants. Text outside the selected
root remains excluded.

Prose whitespace collapses across text nodes, preserving the spaces around inline
emphasis and links without inserting spaces inside words: `<p>The <b>Rust</b>
language</p>` yields `The Rust language`, and `pre<em>fix</em>` yields `prefix`.
Blocks and `br` introduce line breaks; empty wrappers do not add blank lines.
Table rows use line breaks and sibling cells use tabs, including empty cells.
This is plain text, not Markdown or a rectangular table model: row/column spans,
CSS display rules and visual layout are not reconstructed.

Within `pre`, DOM text retains indentation, tabs, blank lines and repeated lines,
including leading/trailing whitespace. Inline `code` outside `pre` follows prose
whitespace rules. HTML parsing still normalizes CRLF and the HTML-defined first
newline in `pre`; this is not byte-for-byte HTML source recovery. Entities decode
once during parsing, so `&amp;lt;` remains the literal text `&lt;`. Unicode joiners
remain intact. Actual repeated blocks and literal `Source:`/`Status:` metadata
lines are retained; the old line-deduplication and prefix filters are removed.

The Unicode-scalar character limit counts code whitespace and generated
separators. Output accumulation stops at the limit, possibly mid-block or
mid-code; no partial scalar is emitted. The parser's response-byte cap and
bounded blocking executor remain unchanged. Empty or whitespace-only retained
output has no extractable content. No new dependency, flag, public field, output
schema or cache-key change is introduced. JSON consumers should expect escaped
newlines/tabs in HTML `content` instead of flattened prose. Cached extractions
keep their old text until expiry or a fresh fetch (omit cache flags for a fresh search-page extraction).

### Ranking decision and validation

The old extractor duplicated headings to imply weights, then immediately removed
those adjacent duplicates. This fix removes that ineffective mechanism. Body
BM25 treats headings as ordinary body tokens once per source occurrence; existing
title/snippet ranking weights stay in the ranking module. Corrected word
boundaries, recovered code and short text, preserved repetition, and a different
retained prefix can deliberately change scores. No new HTML field weighting is
introduced without ranking evidence; broader ranking evaluation remains #78.

`ordered.json` contains authored golden cases for inline links/emphasis,
interleaved headings/paragraphs, code-only pages, tables with empty cells, nested
lists, entities/Unicode joiners, literal metadata/repetition, malformed HTML and
empty documents. Tests assert exact output and every Unicode character cap in
these cases. A deep-wrapper test checks traversal without recursive extraction.
Ranking regression tests compare extracted tokens and BM25 scores with a manually
specified readable corpus, including equal heading/paragraph scores and newly
recoverable code evidence. Local CLI mocks verify exact text/JSON output from the
same golden pages. Temporary skill installation checks the new guidance alongside
current CLI help. No live-web quality or performance improvement is claimed.

## Parser capacity

`KestrelClient` owns an immutable aggregate capacity (default 10), shared across
its clones and all fetch methods, including cache misses. Use
`with_parser_capacity(n)` or `with_transport_and_parser_capacity(transport, n)`
to choose a capacity in `1..=tokio::sync::Semaphore::MAX_PERMITS`; invalid values
return `InvalidRequest` before building HTTP clients. Each call additionally
obeys its own `FetchOptions::parse_concurrency`. Calls with differing limits
cannot resize or replenish the shared pool. The CLI configures its client cap
from `--parse-concurrency`, preserving the flag's existing single-call behavior.

A page first obtains its per-call parser slot, then a shared slot, keeping its
download slot while waiting for both. Queued and running blocking jobs own both
parser slots until body, decoded text and DOM resources have been released.
Cancellation/budget expiry drops waiting bodies and returns without waiting for
already submitted jobs; it does not stop running parsers. Tokio runtime shutdown
may wait for those jobs independently of API return timing. No lock is held
across parsing, and admission always uses the same acquisition order.

Per call, at most `max_concurrency` bodies download or wait for admission.
Across a client's calls, at most its configured capacity of additional bodies
belong to queued/running parsers. Download limits remain per call: applications
must also bound the number of concurrent calls to bound total download memory.
Decoded text/DOM expansion, transport buffers and retained results are additional
memory. Provider parsing is outside this pool (#16). Separate clients and free
fetch functions have independent pools; reuse one client family when requiring
an aggregate parser bound. Cached hits consume no parser capacity.

Deterministic regressions gate an entered parser, assert per-call limits and
shared capacity under overlapping clones, expiry, abort and repeated cached/
uncached calls, then release it and verify progress. A single-blocking-worker
regression covers queued jobs across clones; the existing large-batch regression
continues to check per-call body backpressure. These are resource/lifecycle tests,
not live latency or memory benchmark claims.
