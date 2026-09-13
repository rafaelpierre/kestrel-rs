# Extraction fixtures

These small, authored HTML fixtures represent documentation layouts, not live
captures or copies of third-party text: a book chapter, an API reference, and
an installation/download guide. They combine nested content containers with
advertising, sidebars and navigation. Adjacent `.txt` files are exact expected
extractions at a 20,000-character limit.

`documentation_fixtures_retain_body_and_exclude_chrome` checks the complete
output, including absence of unwanted chrome. These fixtures establish bounded
regression evidence; they do not measure prevalence or general web accuracy.

`ordered.json` adds issue #22's authored source-order, inline spacing, code-only,
table/empty-cell, nested-list, short-answer, entity/Unicode, repetition/metadata,
malformed-markup and empty-document cases. These are exact plain-text expectations,
including newlines and tabs. Unit tests check every character-limit prefix and CLI
mock tests check text and JSON output. The earlier three `.txt` expectations now
use block line breaks; their content/chrome expectations are unchanged.
