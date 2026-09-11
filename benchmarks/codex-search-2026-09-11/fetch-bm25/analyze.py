"""Summarize the completed three-round run against saved earlier baselines."""
import json
import math
import statistics
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent
data = json.loads((ROOT / "results.json").read_text())
baseline = json.loads((ROOT.parent / "summary.json").read_text())
judgments = json.loads((ROOT / "judgments.json").read_text())["judgments"]
scores = {(j["query_id"], j["url"]): j["score"] for j in judgments}
queries = {q["id"]: q for q in data["queries"]}


def summarize(runs):
    seconds = sorted(r["elapsed_ms"] / 1000 for r in runs)
    direct = [sum(scores[r["query_id"], a["url"]] == 2 for a in r["results"]) for r in runs]
    restricted = [r for r in runs if queries[r["query_id"]].get("domain")]
    compliant = 0
    for r in restricted:
        domain = queries[r["query_id"]]["domain"]
        for a in r["results"]:
            host = urlparse(a["url"]).hostname or ""
            compliant += host == domain or host.endswith("." + domain)
    return {"runs": len(runs), "median_seconds": statistics.median(seconds),
            "p95_seconds_nearest_rank": seconds[math.ceil(len(seconds) * .95) - 1],
            "median_process_seconds": statistics.median(r["process_seconds"] for r in runs),
            "direct_results": sum(direct), "precision_at_5": sum(direct) / (5 * len(runs)),
            "runs_with_direct_result": sum(n > 0 for n in direct),
            "empty_runs": sum(not r["results"] for r in runs),
            "returned_results": sum(len(r["results"]) for r in runs),
            "expected_result_slots": 5 * len(runs),
            "nonzero_exit_runs": sum(r["exit_code"] != 0 for r in runs),
            "domain_compliant_results": compliant,
            "domain_expected_slots": 5 * len(restricted)}


summary = {"overall": summarize(data["runs"]),
           "first_round": summarize([r for r in data["runs"] if r["round"] == 1]),
           "repeat_rounds": summarize([r for r in data["runs"] if r["round"] > 1]),
           "saved_baselines": baseline["overall"], "queries": []}
for q in data["queries"]:
    summary["queries"].append({"id": q["id"], "query": q["query"],
                               **summarize([r for r in data["runs"] if r["query_id"] == q["id"]])})
(ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary["overall"], indent=2))

lines = ["# Search benchmark: fetching and BM25 enabled", "",
         "30 Kestrel searches: the same 10 queries, three rounds, executed sequentially. "
         "Built-in search was not rerun, as requested. Its saved five-round baseline is reused.", "",
         "Measured configuration: Kestrel 1.1.1, fanout, provider quorum 1, fetching and BM25 enabled, "
         "top-k 5, up to 15 candidates, 2,000 extracted characters per page, fetch budget 20 seconds, "
         "fetch concurrency **5**. Other settings were CLI defaults.", "",
         "The runner now explicitly uses fetch concurrency **10** for future runs, as requested "
         "after this batch completed. The saved measurements were not rerun at concurrency 10.", "",
         "| Metric | Built-in, saved | Kestrel no-fetch, saved | Kestrel fetch + BM25 |",
         "|---|---:|---:|---:|"]
groups = [baseline["overall"]["builtin"], baseline["overall"]["kestrel"], summary["overall"]]
for label, key in [("Runs", "runs"), ("Median seconds", "median_seconds"),
                   ("p95 seconds (nearest rank)", "p95_seconds_nearest_rank"),
                   ("Direct relevance precision@5", "precision_at_5"),
                   ("Runs with a directly relevant result", "runs_with_direct_result"),
                   ("Empty runs", "empty_runs")]:
    values = [f"{g[key] * 100:.1f}%" if key == "precision_at_5" else f"{g[key]:.3f}" for g in groups]
    lines.append(f"| {label} | " + " | ".join(values) + " |")
lines += ["", "| Query | Built-in median s (saved) | Kestrel no-fetch median s (saved) | Fetch + BM25 median s | Direct hits / 5, mean |",
          "|---|---:|---:|---:|---:|"]
for q in summary["queries"]:
    old = next(a for a in baseline["queries"] if a["id"] == q["id"])
    lines.append(f"| {q['query']} | {old['builtin']['median_seconds']:.3f} | "
                 f"{old['kestrel']['median_seconds']:.3f} | {q['median_seconds']:.3f} | {q['direct_results'] / 3:.1f} |")
lines += ["", "Quality assessment", "",
          "The same 0–2 relevance rubric and earlier judgments for repeated query/URL pairs were used. "
          "New URLs were reviewed using titles, URLs and snippets, keeping fetched body text out of the relevance score. "
          "Score 2 counts as directly relevant; missing top-five slots count as zero. "
          "This is a single-assistant, non-blind assessment of relevance, not a fact-check of page contents.", "",
          "Fetching and BM25 changed ordering and removed candidates but did not resolve the broad-keyword "
          "retrieval failures in this environment. Only the museum-hours query yielded directly relevant results. "
          "The James Webb query returned a James-band discography; PostgreSQL returned definitions of ‘explain’; "
          "TraceQL returned music-tempo pages. These results suggest a query-handling/provider problem, whose cause "
          "has not been diagnosed. This is not evidence that BM25 generally worsens search.", "",
          "The built-in baseline also has usability limitations: translated Codex pages, duplicate documentation "
          "and older PDFs. See ../REPORT.md for the original analysis.", "",
          "Timing limits", "",
          "Complete-call elapsed time includes the shell wrapper and any automatic polling; a monotonic subprocess "
          "timer is saved separately. No model thinking or manual polling gaps were included. Built-in response "
          "size, internal fetching/ranking, and upstream caches are not controllable. The baseline was collected "
          "earlier, and the sample sizes differ (50 vs 30). This is a descriptive local comparison, not a controlled "
          "causal estimate of enabling BM25 or fetching.", "",
          "Files: results.json (raw output and measured settings), judgments.json (per-URL scores), "
          "summary.json (metrics), analyze.py (recompute report), run-kestrel.py (future runs at concurrency 10).", ""]
(ROOT / "REPORT.md").write_text("\n".join(lines))
