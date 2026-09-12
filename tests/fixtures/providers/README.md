Swisscows JSON contains two WebPage items from the live September 11 response. Qwant JSON contains two items from a successful randomized-header trial. Other success fixtures are synthetic wire-contract tests, not evidence of live provider availability. Ecosia markup remains provisional pending a successful live capture. Blocked live responses are retained locally in the Git-ignored benchmarks/investigation-2026-09-11/providers directory.

## Issue #40 feasibility fixtures (2026-09-12)

- `felo-token-required.json`: actual HTTP 400 from the public frontend search
  creation endpoint, without a browser security token.
- `felo-api-unauthorized.json`: actual HTTP 401 from the separate official API;
  request identifier removed.
- `manus-unauthenticated.json`: actual HTTP 401 from official task creation;
  request identifier removed.
- `felo-thread-sanitized.html`: projection of actual completed-thread HTML,
  retaining only query, status, rewrite metadata and the 11 ordered sources.
  All device/visitor/thread identifiers, generated answer and unrelated HTML/props
  were removed. Source snippets are preserved even when unhelpful (cookie text).

These establish the access blockers and completed-page extraction contract, not
production search adapters. No valid-empty success fixture was observed. Synthetic
negative inputs in the tests are explicitly distinct from these live captures.
See `docs/investigations/issue-40-felo-manus.md` for the final no-go decisions.
