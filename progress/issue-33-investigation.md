# Issue #33: Mojeek HTTP 200 challenge classification

Issue: https://github.com/rafaelpierre/kestrel-rs/issues/33
Date: 2026-09-12
Base: main at 95899d1
Branch: codex/issue-33-mojeek-challenges

## Plan and implementation

1. Confirm the observed structure against saved September 12 pilot captures.
   Completed: 23 HTTP 200 captures have a `Captcha` title and `.captcha-wrap`;
   the remaining capture has HTTP 403 and neither marker.
2. Derive a sanitized fixture from the observed markup.
   Completed: retain title and challenge paragraph; remove scripts, generated
   identifiers, request-specific URLs and unrelated page elements. Provenance
   and sanitization are documented in the fixture README.
3. Classify the observed challenge before organic/empty/unknown handling.
   Completed: require both the page title and the JavaScript challenge sentence
   inside `.captcha-wrap > p`. Normalize whitespace and case. Ordinary CAPTCHA
   result titles/snippets remain results.
4. Verify diagnostics and HTTP behavior.
   Implemented: reuse the existing error-to-outcome mapping through a helper;
   local HTTP tests distinguish HTTP 200 challenge from non-retryable HTTP 403.
   Parser tests cover organic results, explicit empty, unknown markup, partial
   challenge markers, and challenge precedence over result/empty markup.
5. Run formatting, lint and the full test suite; inspect the final diff.
   Completed: `cargo fmt --check`, `cargo clippy --all-targets --all-features
   --locked -- -D warnings`, and `cargo test --all-features --locked` pass
   (65 tests). `git diff --check` passes. Captures from the local HTTP tests
   retain statuses 200 and 403 independently, with one request attempt each.
   The detector's exact title/wrapper/message markers were also confirmed
   against all 23 HTTP 200 pilot captures.

## Scope

Transport handling is unchanged: benchmark captures preserve HTTP status before
semantic parsing. `ProviderSearchDiagnostic` has no HTTP-status field; any public
schema expansion belongs with issue #34. This change does not bypass challenges,
change provider defaults, or make claims about global provider availability.
Raw pilot captures remain local and uncommitted. No live provider requests are
needed for this fix.
