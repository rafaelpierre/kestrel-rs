#!/usr/bin/env python3
"""Compare the Python and Rust Kestrel CLIs with matched live-search pairs."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import random
import statistics
import subprocess
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_QUERIES = ROOT / "benchmarks" / "queries.jsonl"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-cli", type=Path, required=True)
    parser.add_argument("--rust-cli", type=Path, default=ROOT / "target/release/kestrel")
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--startup-trials", type=int, default=50)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--timeout", type=int, default=60)
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
    if args.trials < 1 or args.startup_trials < 1:
        parser.error("trial counts must be at least 1")
    for executable in (args.python_cli, args.rust_cli):
        if not executable.is_file():
            parser.error(f"executable not found: {executable}")
    return args


def load_queries(path: Path) -> list[dict[str, str]]:
    queries = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    if not queries or any(not row.get("id") or not row.get("query") for row in queries):
        raise ValueError(f"{path}: every row needs non-empty id and query fields")
    return queries


def percentile(values: list[float], proportion: float) -> float:
    return sorted(values)[round((len(values) - 1) * proportion)]


def summarize(values: list[float]) -> dict[str, float]:
    return {
        "mean": statistics.mean(values),
        "p50": statistics.median(values),
        "p95": percentile(values, 0.95),
        "min": min(values),
        "max": max(values),
    }


def startup_benchmark(executables: dict[str, Path], trials: int) -> dict[str, Any]:
    samples: dict[str, list[float]] = {name: [] for name in executables}
    names = list(executables)
    for trial in range(trials):
        order = names if trial % 2 == 0 else list(reversed(names))
        for name in order:
            started = time.perf_counter()
            completed = subprocess.run(
                [str(executables[name]), "--help"],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            elapsed_ms = (time.perf_counter() - started) * 1000
            if completed.returncode != 0:
                raise RuntimeError(f"{name} --help exited {completed.returncode}")
            samples[name].append(elapsed_ms)
    return {name: {"samples_ms": values, **summarize(values)} for name, values in samples.items()}


def run_search(
    name: str,
    executable: Path,
    task: dict[str, str],
    trial: int,
    engines: list[str],
    mode: str,
    timeout: int,
    artifact_dir: Path,
    extra_args: tuple[str, ...] = (),
) -> dict[str, Any]:
    run_id = uuid.uuid4().hex
    command = [
        str(executable),
        "search",
        task["query"],
        "--output",
        "json",
        "--mode",
        mode,
    ]
    for engine in engines:
        command.extend(("--engine", engine))
    command.extend(extra_args)
    environment = os.environ | {
        "KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR": str(artifact_dir),
        "KESTRELSEARCH_BENCHMARK_RUN_ID": run_id,
    }
    started = time.perf_counter()
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        env=environment,
        text=True,
        timeout=timeout,
    )
    elapsed_ms = (time.perf_counter() - started) * 1000
    if completed.returncode != 0:
        raise RuntimeError(
            f"{name} failed for {task['id']} trial {trial}: {completed.stderr.strip()}"
        )
    results = json.loads(completed.stdout)
    artifact_path = next(artifact_dir.glob(f"{run_id}-*.json"), None)
    if artifact_path is None:
        raise RuntimeError(f"{name} did not write an artifact for run {run_id}")
    artifact = json.loads(artifact_path.read_text())
    timings = artifact.get("timings_ms", {})
    measured_stages_ms = sum(timings.values())
    return {
        "implementation": name,
        "task_id": task["id"],
        "query": task["query"],
        "trial": trial,
        "run_id": run_id,
        "elapsed_ms": elapsed_ms,
        "timings_ms": timings,
        "process_coordination_ms": elapsed_ms - measured_stages_ms,
        "result_count": len(results),
        "returned_chars": artifact.get("returned_chars", 0),
        "urls": [item["url"] for item in results],
        "diagnostics": artifact.get("diagnostics", {}),
    }


def main() -> int:
    args = parse_args()
    queries = load_queries(args.queries)
    executables = {"python": args.python_cli.resolve(), "rust": args.rust_cli.resolve()}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    print(f"Measuring startup ({args.startup_trials} trials per CLI)...", flush=True)
    startup = startup_benchmark(executables, args.startup_trials)
    runs: list[dict[str, Any]] = []
    randomizer = random.Random(args.seed)
    with tempfile.TemporaryDirectory(prefix="kestrel-cli-benchmark-") as directory:
        artifact_dir = Path(directory)
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            for trial in range(1, args.trials + 1):
                shuffled = list(queries)
                randomizer.shuffle(shuffled)
                for task in shuffled:
                    futures = {
                        name: executor.submit(
                            run_search,
                            name,
                            executable,
                            task,
                            trial,
                            args.engines,
                            args.mode,
                            args.timeout,
                            artifact_dir,
                        )
                        for name, executable in executables.items()
                    }
                    pair = {name: future.result() for name, future in futures.items()}
                    runs.extend(pair.values())
                    for name, run in pair.items():
                        print(
                            f"[{trial}/{args.trials}] {task['id']} {name}: "
                            f"{run['elapsed_ms']:.0f} ms",
                            flush=True,
                        )
    result = {
        "configuration": {
            "python_cli": str(executables["python"]),
            "rust_cli": str(executables["rust"]),
            "queries": str(args.queries.resolve()),
            "trials": args.trials,
            "startup_trials": args.startup_trials,
            "seed": args.seed,
            "engines": args.engines,
            "mode": args.mode,
        },
        "startup": startup,
        "runs": runs,
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Results: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
