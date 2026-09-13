#!/usr/bin/env python3
"""Alternate fresh normal/preloaded-root processes; retain all client builds."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import time

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    records = []
    for trial in range(10):
        for mode in (["normal", "preload"] if trial % 2 == 0 else ["preload", "normal"]):
            start = time.perf_counter_ns()
            result = subprocess.run([str(args.binary.resolve()), mode], capture_output=True, text=True, check=True, timeout=30)
            records.append({"trial": trial, "mode": mode,
                            "external_seconds": (time.perf_counter_ns() - start) / 1e9,
                            "builds": [json.loads(line) for line in result.stdout.splitlines()],
                            "stderr": result.stderr})
    (args.output / "runs.json").write_text(json.dumps(records, indent=2))
    (args.output / "binary-sha256.txt").write_text(hashlib.sha256(args.binary.read_bytes()).hexdigest() + "\n")
