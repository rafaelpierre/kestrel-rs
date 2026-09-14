#!/usr/bin/env python3
"""Compare fresh engine-scoped CLI startup using the #80 subprocess harness."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
from datetime import datetime, timezone

from run import sha, trial
from summarize import distribution


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--trials', type=int, default=20)
    args = parser.parse_args()
    if args.trials < 1:
        parser.error('trials must be positive')
    args.output.mkdir(parents=True, exist_ok=False)
    binaries = {'before': args.before.resolve(), 'after': args.after.resolve()}
    env = {k: v for k, v in os.environ.items() if not k.startswith(('KESTRELSEARCH_', 'BUDGET_'))}
    env['KESTRELSEARCH_OTEL_ENABLED'] = 'false'
    metadata = {'started_at': datetime.now(timezone.utc).isoformat(),
                'platform': platform.platform(), 'trials': args.trials,
                'binaries': {k: {'path': str(v), 'sha256': sha(v)} for k, v in binaries.items()},
                'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                'policy': 'Sequential fresh processes, seed 93 shuffled six-cell blocks; 1 ns search budget; no fetching/ranking/caches/traces; unchanged TLS/proxy environment. Default all-provider and explicit Yahoo controls. Median and nearest-rank p95, milliseconds. No live network performance claim.'}
    (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    rng = random.Random(93)
    records = []
    for round_number in range(args.trials):
        cells = [(variant, engine) for variant in binaries for engine in ('bing', 'yahoo', 'default')]
        rng.shuffle(cells)
        for variant, engine in cells:
            command = [str(binaries[variant]), 'search', 'startup control', '--search-budget',
                       '0.000000001', '--no-fetch', '--no-rank', '--output', 'json']
            if engine != 'default':
                command += ['--engine', engine]
            label = f'{round_number}-{variant}-{engine}'
            row = trial(command, env, args.output, label)
            row.update(variant=variant, engine=engine, round=round_number)
            records.append(row)
            (args.output / 'runs.json').write_text(json.dumps(records, indent=2))
            stdout = (args.output / row['stdout_file']).read_text()
            stderr = (args.output / row['stderr_file']).read_text()
            if row['returncode'] != 1 or stdout.strip() or 'Every search failed: search deadline exceeded' not in stderr:
                raise RuntimeError(f'{label}: expected deadline failure with empty stdout; inspect retained output')
        print(f'Completed startup block {round_number + 1}/{args.trials}', flush=True)
    summary = {f'{variant}/{engine}': distribution([r['external_seconds'] for r in records
               if r['variant'] == variant and r['engine'] == engine])
               for variant in binaries for engine in ('bing', 'yahoo', 'default')}
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
