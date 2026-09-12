#!/usr/bin/env python3
"""Run bounded Bing comparisons and score explicit, per-result relevance judgments."""
import argparse
from collections import defaultdict
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import shutil
import time

ROOT = Path(__file__).resolve().parents[1]


def load_runs(paths):
    runs = []
    for path in paths:
        artifact = json.loads(Path(path).read_text())
        if artifact.get('schema') != 1:
            raise ValueError(f'{path}: unsupported schema')
        if not artifact.get('completed') or len(artifact['runs']) != artifact.get('expected_rows'):
            raise ValueError(f'{path}: incomplete schedule; do not silently exclude missing searches')
        for row in artifact['runs']:
            runs.append(dict(row, source=str(path)))
    return runs


def judgment_key(query, result):
    # A URL alone cannot identify snippets or query intent across observations.
    data = [query, result['url'], result['title'], result.get('snippet', '')]
    return hashlib.sha256(json.dumps(data, ensure_ascii=False).encode()).hexdigest()


def judgment_template(runs):
    pool = {}
    for row in runs:
        for result in row['results'][:5]:
            key = judgment_key(row['query'], result)
            pool[key] = dict(query=row['query'], url=result['url'], title=result['title'],
                             snippet=result.get('snippet', ''), relevant=None, reason='')
    return {'rubric': 'Relevant means addresses the specific query intent, not merely shared words. '
            'Judge the returned title/snippet and destination; this does not validate page correctness. '
            'Use true/false with a reason. Leave uncertain entries null.', 'judgments': pool}


def percentile(values, percent):
    return sorted(values)[max(0, math.ceil(len(values) * percent) - 1)] if values else None


def score(runs, judgments):
    groups = defaultdict(list)
    for row in runs:
        groups[row['variant']].append(row)
    summaries = {}
    for variant, rows in groups.items():
        nonempty = useful = unknown_runs = relevant = missing = 0
        known_useful = 0
        for row in rows:
            top = row['results'][:5]
            nonempty += bool(top)
            labels = []
            for result in top:
                j = judgments.get(judgment_key(row['query'], result), {})
                label = j.get('relevant')
                if label is not None and type(label) is not bool:
                    raise ValueError('relevant must be boolean or null')
                if label is not None and not j.get('reason', '').strip():
                    raise ValueError('every judgment needs a reason')
                labels.append(label)
            hit = any(v is True for v in labels)
            unknown = any(v is None for v in labels)
            known_useful += hit
            unknown_runs += unknown and not hit
            useful += hit
            relevant += sum(v is True for v in labels)
            missing += sum(v is None for v in labels)
        count = len(rows)
        summaries[variant] = {
            'scheduled': count, 'nonempty': nonempty,
            'empty_or_error': count - nonempty,
            'errors': sum(bool(r.get('error')) for r in rows),
            'over_budget': sum(r['elapsed_ms'] > r['budget_ms'] for r in rows),
            'unjudged_results': missing,
            'conditional_precision_at_5': relevant / (5 * nonempty) if nonempty and not missing else None,
            'usable_coverage': useful / count if not unknown_runs else None,
            'usable_coverage_bounds': [known_useful / count, (known_useful + unknown_runs) / count],
            'p50_ms': percentile([r['elapsed_ms'] for r in rows], .5),
            'p95_ms': percentile([r['elapsed_ms'] for r in rows], .95),
            'provider_cancellations': sum(r.get('cancelled', 0) for r in rows),
            'runs_missing_provider_diagnostics': sum(
                not r['variant'].startswith('bing-') and 'providers' not in r for r in rows),
            'observed_completed_provider_attempts': sum(sum(1 + p.get('retries', 0) for p in r.get('providers', [])
                                                  if p.get('outcome') != 'cancelled_quorum') for r in rows),
            'isolated_attempts': sum(r.get('attempts', 0) for r in rows),
        }
    return summaries


def run(args):
    if args.windows < 1 or args.interval < 0:
        raise ValueError('windows must be positive and interval nonnegative')
    args.output.mkdir(parents=True, exist_ok=False)
    start = time.monotonic()
    schedule = {'created_at': datetime.now(timezone.utc).isoformat(),
                'windows': args.windows, 'interval_seconds': args.interval,
                'query_manifest': 'benchmarks/bing-fidelity/queries.json', 'executions': []}
    build = subprocess.run(['cargo', 'test', '--lib', '--no-run', '--message-format=json'],
                           cwd=ROOT, check=True, capture_output=True, text=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    executable = next(Path(a['executable']) for a in artifacts
                      if a.get('reason') == 'compiler-artifact' and a.get('executable')
                      and a.get('profile', {}).get('test') and 'lib' in a.get('target', {}).get('kind', []))
    # Freeze the tested executable so later edits/builds cannot alter a schedule.
    binary = (args.output / 'experiment-test').resolve()
    shutil.copy2(executable, binary)
    schedule['executable_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
    schedule['query_manifest_sha256'] = hashlib.sha256(
        (ROOT / schedule['query_manifest']).read_bytes()).hexdigest()
    schedule['encoding_only'] = args.encodings
    start = time.monotonic()
    for window in range(args.windows):
        target = start + window * args.interval
        time.sleep(max(0, target - time.monotonic()))
        output = (args.output / f'window-{window + 1}').resolve()
        env = dict(os.environ, KESTREL_BING_EXPERIMENT_DIR=str(output))
        if args.raw:
            env['KESTREL_BING_RAW_DIR'] = str(output / 'raw')
        else:
            env.pop('KESTREL_BING_RAW_DIR', None)
        if args.encodings:
            env['KESTREL_BING_ENCODING_ONLY'] = '1'
        else:
            env.pop('KESTREL_BING_ENCODING_ONLY', None)
        # Do not accidentally activate unrelated raw tracing from a parent shell.
        env.pop('KESTRELSEARCH_PROVIDER_TRACE_DIR', None)
        invocation = {'window': window + 1, 'started_at': datetime.now(timezone.utc).isoformat()}
        proc = subprocess.run([str(binary), 'search::bing_fidelity::capture_live_matrix',
                               '--ignored', '--exact', '--nocapture'], cwd=ROOT, env=env)
        invocation['exit_code'] = proc.returncode
        schedule['executions'].append(invocation)
        (args.output / 'schedule.json').write_text(json.dumps(schedule, indent=2))
        if proc.returncode:
            raise RuntimeError('experiment failed; partial evidence retained, do not score as a complete schedule')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    live = sub.add_parser('run')
    live.add_argument('--output', required=True, type=Path)
    live.add_argument('--windows', type=int, default=1)
    live.add_argument('--interval', type=float, default=300)
    live.add_argument('--encodings', action='store_true', help='compare + and %%20 spaces only')
    live.add_argument('--raw', action='store_true', help='also retain unredacted local HTML; do not commit it')
    for name in ['template', 'score']:
        p = sub.add_parser(name)
        p.add_argument('runs', nargs='+', type=Path)
        p.add_argument('--output', required=True, type=Path)
        if name == 'score':
            p.add_argument('--judgments', required=True, type=Path)
    args = parser.parse_args()
    if args.command == 'run':
        run(args)
        return
    runs = load_runs(args.runs)
    data = judgment_template(runs) if args.command == 'template' else score(
        runs, json.loads(args.judgments.read_text())['judgments'])
    with args.output.open('x') as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write('\n')


if __name__ == '__main__':
    main()
