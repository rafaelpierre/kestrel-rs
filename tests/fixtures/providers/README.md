Swisscows JSON contains two WebPage items from the live September 11 response. Qwant JSON contains two items from a successful randomized-header trial. Other success fixtures are synthetic wire-contract tests, not evidence of live provider availability. Ecosia markup remains provisional pending a successful live capture. Blocked live responses are retained locally in the Git-ignored benchmarks/investigation-2026-09-11/providers directory.

`mojeek-challenge.html` is minimized from the September 12, 2026 issue #29
pilot's HTTP 200 response. All 23 HTTP 200 Mojeek captures shared the `Captcha`
title and `.captcha-wrap` JavaScript challenge message. The fixture retains
those elements and the document/body attributes, while removing scripts,
provider-generated identifiers, request-specific URLs, navigation, styles,
and unrelated metadata. The raw captures remain local and uncommitted.

`bing-unrelated.html` is synthetic. Its full-query document title and unrelated organic entries exercise faithful parsing and the current nonempty fallback/quorum policy; it does not establish live Bing behavior or semantic relevance.

`bing-live-quoted.html`, `bing-live-tokio.html` and `bing-live-site.html` are sanitized first-two-result excerpts from September 12 live responses. See `bing-live-provenance.json` for context and response hashes. They preserve result markup/text, decode destinations and remove tracking/active content. The quoted/site failures and the successful Tokio response exercise parsing; they do not establish a global provider failure or its cause. Use `benchmarks/sanitize_bing_fixture.py` to create reviewed excerpts from local raw captures.
