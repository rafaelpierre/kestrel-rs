# Page chrome matching

Direct `fetch` and search candidate fetching share the bounded HTML extractor
for HTML/XHTML responses. `text/plain` bypasses DOM cleanup and preserves literal
text; see [plain-text decoding and limits](cli-arguments.md#plain-text-responses).
Before selecting and extracting body text, it removes `script`, `style`, `nav`,
`header`, `footer`, `aside`, and `form` elements as before.

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
content container using an exact clutter name can still be removed. Structural
extraction still uses the existing selected tags and limits. See the bounded
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
