"""Recompute benchmark metrics and export every scored top-five result."""
import json
import math
import statistics
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent
data = json.loads((ROOT / "results.json").read_text())
assessment = json.loads((ROOT / "judgments.json").read_text())
judgments = {(j["query_id"], j["url"]): j for j in assessment["judgments"]}
queries = {q["id"]: q for q in data["queries"]}


def summarize(runs):
    latencies = sorted(r["elapsed_ms"] / 1000 for r in runs)
    scores = [[judgments[r["query_id"], a["url"]]["score"] for a in r["results"]]
              for r in runs]
    restricted = [r for r in runs if queries[r["query_id"]].get("domain")]
    matched = 0
    for r in restricted:
        domain = queries[r["query_id"]]["domain"]
        for a in r["results"]:
            host = urlparse(a["url"]).hostname or ""
            matched += host == domain or host.endswith("." + domain)
    return {
        "runs": len(runs),
        "median_seconds": statistics.median(latencies),
        "p95_seconds_nearest_rank": latencies[math.ceil(len(latencies) * 0.95) - 1],
        "mean_seconds": statistics.mean(latencies),
        "direct_results": sum(s == 2 for ss in scores for s in ss),
        "expected_result_slots": 5 * len(runs),
        "precision_at_5": sum(s == 2 for ss in scores for s in ss) / (5 * len(runs)),
        "graded_relevance": sum(sum(ss) for ss in scores) / (10 * len(runs)),
        "runs_with_direct_result": sum(2 in ss for ss in scores),
        "empty_runs": sum(not r["results"] for r in runs),
        "nonzero_exit_runs": sum(r.get("exit_code", 0) != 0 for r in runs),
        "domain_compliant_results": matched,
        "domain_expected_slots": 5 * len(restricted),
    }


summary = {"overall": {}, "first_round": {}, "repeat_rounds": {}, "queries": []}
for engine in ("builtin", "kestrel"):
    runs = [r for r in data["runs"] if r["engine"] == engine]
    summary["overall"][engine] = summarize(runs)
    summary["first_round"][engine] = summarize([r for r in runs if r["round"] == 1])
    summary["repeat_rounds"][engine] = summarize([r for r in runs if r["round"] > 1])
for query in data["queries"]:
    row = {"id": query["id"], "query": query["query"]}
    for engine in ("builtin", "kestrel"):
        row[engine] = summarize([r for r in data["runs"]
                                 if r["engine"] == engine and r["query_id"] == query["id"]])
    summary["queries"].append(row)

(ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary["overall"], indent=2))
for q in summary["queries"]:
    print(q["id"], q["builtin"]["median_seconds"], q["kestrel"]["median_seconds"],
          q["builtin"]["direct_results"] / 5, q["kestrel"]["direct_results"] / 5)
