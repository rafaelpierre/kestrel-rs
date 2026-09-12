# Felo / Manus feasibility evidence

The final report is [issue-40-felo-manus.md](../../docs/investigations/issue-40-felo-manus.md).

- `queries.json`: six shared inputs; the Unicode stress case is unjudged.
- `observations.jsonl`: 96 curated scheduled observations, including ten cooldown
  skips. Results are top-five projections; `result_count` preserves original size.
- `judgments.json`: 44 manually reviewed query/URL relevance labels. These judge
  snippets/titles/URLs, not page-body accuracy or comprehensive freshness.
- `summary.json`: reproducible statistics from those observations and labels.
- `felo_extract.py`: offline proof of concept for completed Felo thread HTML.
- `test_felo_extract.py`: real-fixture extraction and synthetic negative tests.

Run `python3 benchmarks/provider-feasibility/summarize.py benchmarks/provider-feasibility/observations.jsonl`
from the repository root to reproduce the summary. Unseen query/URL pairs require
manual judgments. Run the Rust `provider_feasibility` example for new live trials;
wait until any provider cooldown expires before starting another window/process.
The initial window did not honor Retry-After; the final runner corrects that defect.
The report preserves this limitation rather than treating the rate as natural
provider reliability. Raw captures are local ignored investigation artifacts.
