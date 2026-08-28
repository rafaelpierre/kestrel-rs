#!/usr/bin/env python3
"""Compare Rust candidate selection with and without snippet pre-ranking."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import random
import statistics
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any

from cli_compare import DEFAULT_QUERIES, ROOT, load_queries, percentile, run_search


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-cli", type=Path, default=ROOT / "target/release/kestrel")
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--top-k", type=int, default=5)
    parser.add_argument("--fetch-candidates", type=int, default=15)
    parser.add_argument("--provider-quorum", type=int)
    parser.add_argument(
        "--engine",
        action="append",
        dest="engines",
        choices=("duckduckgo", "bing", "yahoo"),
    )
    parser.add_argument("--mode", choices=("fallback", "fanout"), default="fanout")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.engines = args.engines or ["duckduckgo", "bing", "yahoo"]
    if args.trials < 1 or args.top_k < 1 or args.fetch_candidates < 1:
        parser.error("trials, top-k, and fetch-candidates must be at least 1")
    if args.fetch_candidates < args.top_k:
        parser.error("fetch-candidates must be greater than or equal to top-k")
    if args.provider_quorum is not None and not 1 <= args.provider_quorum <= len(args.engines):
        parser.error("provider-quorum must be between 1 and the selected engine count")
    if not args.rust_cli.is_file():
        parser.error(f"executable not found: {args.rust_cli}")
    return args


def phase_ms(run: dict[str, Any], phase: str) -> float:
    if phase != "parse":
        return float(run["timings_ms"].get(phase, 0))
    pages = run["diagnostics"].get("fetch") or {}
    return float(sum(page.get("parse_ms", 0) for page in pages.get("pages", [])))


def fetch_value(run: dict[str, Any], key: str) -> int:
    fetch = run["diagnostics"].get("fetch") or {}
    if key == "requests":
        return sum(
            1
            for page in fetch.get("pages", [])
            if page.get("outcome") != "cache_hit"
        )
    return int(fetch.get(key, 0))


def distribution(values: list[float]) -> dict[str, float]:
    return {
        "mean": statistics.mean(values),
        "p50": statistics.median(values),
        "p95": percentile(values, 0.95),
        "min": min(values),
        "max": max(values),
    }


def arm_summary(runs: list[dict[str, Any]]) -> dict[str, Any]:
    phases = ("search", "fetch", "parse", "rank")
    outcomes: Counter[str] = Counter()
    provider_successes = 0
    for run in runs:
        provider_successes += sum(
            provider.get("success", False)
            for provider in run["diagnostics"].get("providers", [])
        )
        outcomes.update(((run["diagnostics"].get("fetch") or {}).get("outcomes") or {}))
    return {
        "elapsed_ms": distribution([run["elapsed_ms"] for run in runs]),
        "phases_ms": {
            phase: distribution([phase_ms(run, phase) for run in runs])
            for phase in phases
        },
        "result_count": distribution([float(run["result_count"]) for run in runs]),
        "returned_chars": distribution([float(run["returned_chars"]) for run in runs]),
        "requests": sum(fetch_value(run, "requests") for run in runs),
        "response_bytes": sum(fetch_value(run, "response_bytes") for run in runs),
        "provider_successes": provider_successes,
        "provider_cancellations": sum(
            run["diagnostics"].get("provider_cancellations", 0) for run in runs
        ),
        "fetch_outcomes": dict(sorted(outcomes.items())),
    }


def overlap(left: list[str], right: list[str]) -> float:
    left_set, right_set = set(left), set(right)
    union = left_set | right_set
    return len(left_set & right_set) / len(union) if union else 1.0


def summarize(runs: list[dict[str, Any]]) -> dict[str, Any]:
    by_arm = {
        arm: [run for run in runs if run["implementation"] == arm]
        for arm in ("baseline", "pre_rank")
    }
    indexed = {
        (run["task_id"], run["trial"], run["implementation"]): run for run in runs
    }
    pairs = []
    for task_id, trial, arm in sorted(indexed):
        if arm != "baseline":
            continue
        baseline = indexed[(task_id, trial, "baseline")]
        pre_rank = indexed[(task_id, trial, "pre_rank")]
        pairs.append(
            {
                "task_id": task_id,
                "trial": trial,
                "url_jaccard": overlap(baseline["urls"], pre_rank["urls"]),
                "common_urls": len(set(baseline["urls"]) & set(pre_rank["urls"])),
                "elapsed_delta_ms": pre_rank["elapsed_ms"] - baseline["elapsed_ms"],
                "returned_chars_delta": pre_rank["returned_chars"]
                - baseline["returned_chars"],
            }
        )
    return {
        "arms": {arm: arm_summary(arm_runs) for arm, arm_runs in by_arm.items()},
        "paired": {
            "pre_rank_faster": sum(pair["elapsed_delta_ms"] < 0 for pair in pairs),
            "pair_count": len(pairs),
            "elapsed_delta_ms": distribution(
                [pair["elapsed_delta_ms"] for pair in pairs]
            ),
            "returned_chars_delta": distribution(
                [float(pair["returned_chars_delta"]) for pair in pairs]
            ),
            "url_jaccard": distribution([pair["url_jaccard"] for pair in pairs]),
            "pairs": pairs,
        },
    }


def main() -> int:
    args = parse_args()
    queries = load_queries(args.queries)
    executable = args.rust_cli.resolve()
    common_args = (
        "--top-k",
        str(args.top_k),
        "--fetch-candidates",
        str(args.fetch_candidates),
    )
    if args.provider_quorum is not None:
        common_args += ("--provider-quorum", str(args.provider_quorum))
    arms = {"baseline": common_args, "pre_rank": common_args + ("--pre-rank",)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    runs: list[dict[str, Any]] = []
    randomizer = random.Random(args.seed)
    with tempfile.TemporaryDirectory(prefix="kestrel-pre-rank-ablation-") as directory:
        artifact_dir = Path(directory)
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            for trial in range(1, args.trials + 1):
                shuffled = list(queries)
                randomizer.shuffle(shuffled)
                for task in shuffled:
                    futures = {
                        arm: executor.submit(
                            run_search,
                            arm,
                            executable,
                            task,
                            trial,
                            args.engines,
                            args.mode,
                            args.timeout,
                            artifact_dir,
                            extra_args,
                        )
                        for arm, extra_args in arms.items()
                    }
                    pair = {arm: future.result() for arm, future in futures.items()}
                    runs.extend(pair.values())
                    print(
                        f"[{trial}/{args.trials}] {task['id']}: "
                        f"baseline={pair['baseline']['elapsed_ms']:.0f} ms, "
                        f"pre-rank={pair['pre_rank']['elapsed_ms']:.0f} ms",
                        flush=True,
                    )
    result = {
        "configuration": {
            "rust_cli": str(executable),
            "queries": str(args.queries.resolve()),
            "trials": args.trials,
            "seed": args.seed,
            "engines": args.engines,
            "mode": args.mode,
            "top_k": args.top_k,
            "fetch_candidates": args.fetch_candidates,
            "provider_quorum": args.provider_quorum,
        },
        "summary": summarize(runs),
        "runs": runs,
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Results: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
