#!/usr/bin/env python3
"""Bounded Yahoo observation; raw artifacts stay local. No policy rollout."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def run(binary, directory, label, arguments):
    target = directory / label
    target.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env.update(KESTRELSEARCH_PROVIDER_TRACE_DIR=str(target / 'traces'),
               KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR=str(target / 'artifacts'),
               KESTRELSEARCH_BENCHMARK_RUN_ID=label)
    argv = [str(binary), *arguments]
    start = datetime.datetime.now(datetime.timezone.utc).isoformat()
    clock = time.monotonic()
    with (target / 'stdout.json').open('w') as out, (target / 'stderr.txt').open('w') as err:
        try:
            code = subprocess.run(argv, env=env, stdout=out, stderr=err, timeout=30).returncode
        except subprocess.TimeoutExpired:
            code = 'outer_timeout'
    record = dict(argv=argv, started_at=start, exit_code=code,
                  wall_seconds=time.monotonic() - clock)
    (target / 'command.json').write_text(json.dumps(record, indent=2) + '\n')
    print(label, code, round(record['wall_seconds'], 3), flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    dataset = Path('benchmarks/codex-search-2026-09-11/queries.json')
    metadata = dict(revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                    tree=subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], text=True).strip(),
                    binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
                    dataset_sha256=hashlib.sha256(dataset.read_bytes()).hexdigest(),
                    policy='Two sequential sweeps, first three manifest queries; fresh CLI/client per call; '
                           'Yahoo only; 3s search budget; 30s outer timeout; no fetch/rank/cache; '
                           'random production profile captured; 250ms inter-call pause. '
                           'All attempts retained, application sends are not redirect hops. '
                           'No causal latency or global availability claim. Median is middle/mean of middle two.')
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    (output / 'tracked.patch').write_bytes(subprocess.check_output(['git', 'diff', 'HEAD', '--binary']))
    queries = json.loads(dataset.read_text())[:3]
    for sweep in range(2):
        for row in queries:
            run(binary, output, f"s{sweep + 1}-{row['id']}",
                ['search', row['query'], '-e', 'yahoo', '--no-fetch', '--no-rank',
                 '--search-budget', '3', '--min-results', '100', '-k', '20', '--output', 'json'])
            time.sleep(.25)


if __name__ == '__main__':
    main()
