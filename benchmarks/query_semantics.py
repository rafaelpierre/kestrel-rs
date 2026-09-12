#!/usr/bin/env python3
"""Opt-in CLI smoke matrix; outcomes are observations, not relevance judgments.

Example (build first):
  cargo build
  python3 benchmarks/query_semantics.py --binary target/debug/kestrel \
    --output benchmarks/results/query-semantics.json

Use --stages no-fetch for retrieval-only checks. Default queries are a phrase,
conjunction and grouped expression. No raw provider bodies or headers are saved.
"""
import argparse
import concurrent.futures
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import time

ENGINES = ["duckduckgo", "bing", "yahoo", "dogpile", "ecosia", "swisscows", "yep", "qwant", "mojeek"]
STAGES = {"default": [], "no-rank": ["--no-rank"], "no-fetch": ["--no-rank", "--no-fetch"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--engines", nargs="+", choices=ENGINES, default=ENGINES)
    parser.add_argument("--stages", nargs="+", choices=STAGES, default=list(STAGES))
    parser.add_argument("--query", action="append")
    parser.add_argument("--syntax", choices=["portable", "native"], default="portable")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Reserve before making requests; never overwrite an earlier evidence run.
    with args.output.open("x") as output:
        evidence = {"started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                    "syntax": args.syntax, "complete": False, "runs": []}
        json.dump(evidence, output, indent=2)
    queries = args.query or ['"machine learning"', 'machine AND learning', '("machine learning" OR "deep learning") -jobs']
    cases = [(engine, stage, query) for engine in args.engines for stage in args.stages for query in queries]

    def run(case):
        engine, stage, query = case
        command = [str(binary), "search", query, "--engine", engine, "--query-syntax", args.syntax,
                   "--search-budget", "5", "--fetch-budget", "2", "--timeout", "3", "--top-k", "3",
                   "--output", "json", *STAGES[stage]]
        start = time.monotonic()
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=25)
            row = {"exit_code": result.returncode, "stderr": result.stderr}
            if result.returncode:
                row["outcome"] = "error"
            else:
                payload = json.loads(result.stdout)
                row["results"] = payload["results"] if isinstance(payload, dict) else payload
                row["outcome"] = "results" if row["results"] else "empty"
        except (subprocess.TimeoutExpired, json.JSONDecodeError) as error:
            row = {"outcome": "harness_error", "error": str(error)}
        return {"engine": engine, "stage": stage, "query": query,
                "elapsed_seconds": round(time.monotonic() - start, 3), **row}

    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        for row in executor.map(run, cases):
            evidence["runs"].append(row)
            args.output.write_text(json.dumps(evidence, indent=2))
            print(row["engine"], row["stage"], repr(row["query"]), row["outcome"], flush=True)
    evidence["complete"] = True
    args.output.write_text(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()
