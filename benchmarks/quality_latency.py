#!/usr/bin/env python3
"""Sequential, resumable retrieval experiments; never modifies historical baselines."""
import argparse
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_QUERIES = ROOT / 'benchmarks/codex-search-2026-09-11/queries.json'


def configurations(stage, engine):
    single = lambda e: ['--engine', e]
    fanout = ['--mode', 'fanout']
    nofetch = ['--no-fetch', '--no-rank']
    if stage == 'retrieval':
        return [(e, single(e) + nofetch) for e in ['duckduckgo', 'bing', 'yahoo']] + [
            ('fanout-q1', fanout + ['--provider-quorum', '1'] + nofetch),
            ('fanout-q2', fanout + ['--provider-quorum', '2'] + nofetch),
            ('fanout-all', fanout + nofetch)]
    if stage == 'providers':
        return [(e, single(e) + nofetch) for e in ['dogpile', 'ecosia', 'swisscows', 'yep', 'qwant', 'mojeek']]
    base = single(engine)
    if stage == 'ranking':
        return [(p, base + ['--ranking-policy', p, '--fetch-candidates', '15', '--fetch-budget', '20'])
                for p in ['provider', 'snippet', 'body', 'hybrid', 'rrf']]
    if stage == 'candidates':
        return [(f'candidates-{n}-prerank-{pre}', base + ['--ranking-policy', 'hybrid', '--fetch-candidates', str(n),
                 '--fetch-budget', '20'] + (['--pre-rank'] if pre else [])) for n in [5, 10, 15] for pre in [False, True]]
    if stage == 'concurrency':
        return [(f'concurrency-{n}', base + ['--ranking-policy', 'hybrid', '--concurrency', str(n), '--fetch-budget', '20']) for n in [5, 10]]
    if stage == 'fetch-budget':
        return [(f'budget-{n}', base + ['--ranking-policy', 'hybrid', '--fetch-budget', str(n)]) for n in [1, 2, 5, 20]]
    if stage == 'search-budget':
        return [(f'search-{n}', fanout + nofetch + (['--search-budget', str(n)] if n else [])) for n in [0, 1, 2, 3]]
    if stage == 'swisscows':
        return [('swisscows', base + nofetch)]
    raise ValueError(stage)


def capture(binary, query, args, directory, timeout, trace):
    directory.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env['KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR'] = str(directory)
    env['KESTRELSEARCH_BENCHMARK_RUN_ID'] = 'run'
    if trace:
        env['KESTRELSEARCH_PROVIDER_TRACE_DIR'] = str(directory / 'providers')
    command = [str(binary), 'search', query, *args, '-k', '5', '--output', 'json']
    start = time.perf_counter()
    try:
        proc = subprocess.run(command, capture_output=True, text=True, timeout=timeout, env=env)
        stdout, stderr, status = proc.stdout, proc.stderr, proc.returncode
    except subprocess.TimeoutExpired as error:
        def text(v):
            return v.decode(errors='replace') if isinstance(v, bytes) else v or ''
        stdout, stderr, status = text(error.stdout), text(error.stderr), 124
    elapsed = time.perf_counter() - start
    try:
        results = json.loads(stdout)
        if isinstance(results, dict):
            results = results["results"]
        if not isinstance(results, list):
            raise ValueError('expected result list')
    except (ValueError, TypeError):
        results = []
        if status == 0:
            status = 65
    artifacts = [json.loads(p.read_text()) for p in directory.glob('run-*.json')]
    return dict(command=command, process_seconds=elapsed, exit_code=status, stdout=stdout,
                stderr=stderr, results=results, artifacts=artifacts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--queries', type=Path, default=DEFAULT_QUERIES)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--stage', choices=['retrieval', 'providers', 'ranking', 'candidates', 'concurrency', 'fetch-budget', 'search-budget', 'swisscows'], default='retrieval')
    parser.add_argument('--engine', default='swisscows')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=50)
    parser.add_argument('--trace', action='store_true')
    parser.add_argument('--search-budget', type=float, help='Apply a common search deadline; recorded separately from timeout')
    parser.add_argument('--fetch-budget', type=float, help='Override common fetch budget outside its ablation stage')
    parser.add_argument('--limit', type=int)
    parser.add_argument('--resume', action='store_true')
    parser.add_argument('--cache', choices=['off', 'cold', 'warm'], default='off')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.rounds < 1 or args.timeout <= 0:
        parser.error('rounds and timeout must be positive')
    if args.output.exists() and not args.resume:
        parser.error('output exists; use a new directory or --resume')
    args.output.mkdir(parents=True, exist_ok=True)
    args.output = args.output.resolve()
    raw = json.loads(args.queries.read_text())
    queries = (raw if isinstance(raw, list) else raw['queries'])[:args.limit]
    configs = configurations(args.stage, args.engine)
    if args.search_budget is not None:
        if args.search_budget <= 0 or args.stage == 'search-budget':
            parser.error('positive common search budget only outside the search-budget stage')
        configs = [(name, flags + ['--search-budget', str(args.search_budget)]) for name, flags in configs]
    if args.fetch_budget is not None:
        if args.fetch_budget <= 0 or args.stage == 'fetch-budget':
            parser.error('positive common fetch budget only outside the fetch-budget stage')
        adjusted = []
        for name, flags in configs:
            flags = list(flags)
            if '--fetch-budget' in flags:
                at = flags.index('--fetch-budget')
                del flags[at:at+2]
            adjusted.append((name, flags + ['--fetch-budget', str(args.fetch_budget)]))
        configs = adjusted
    source_hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                     for p in sorted((ROOT/'src').glob('*.rs'))}
    metadata = dict(source_sha256=source_hashes, binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
                    commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    tracked_diff_sha256=hashlib.sha256(subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT)).hexdigest(),
                    query_sha256=hashlib.sha256(args.queries.read_bytes()).hexdigest(),
                    stage=args.stage, rounds=args.rounds, cache=args.cache, configs=configs,
                    clock='monotonic process wall time; not native tool complete-call latency',
                    timeout=args.timeout, query_ids=[q['id'] for q in queries])
    meta_path = args.output / 'metadata.json'
    if meta_path.exists() and json.loads(meta_path.read_text()) != json.loads(json.dumps(metadata)):
        parser.error('resume metadata differs; choose a new output directory')
    meta_path.write_text(json.dumps(metadata, indent=2) + '\n')
    frozen = args.output / '.binary' / 'kestrel'
    frozen.parent.mkdir(exist_ok=True)
    if not frozen.exists():
        shutil.copy2(binary, frozen)
    if hashlib.sha256(frozen.read_bytes()).hexdigest() != metadata['binary_sha256']:
        parser.error('frozen binary differs; use a new output directory')
    binary = frozen
    output = args.output / 'runs.jsonl'
    completed = {r['run_id'] for r in map(json.loads, output.read_text().splitlines())} if output.exists() else set()
    with output.open('a') as stream:
        for round_ in range(1, args.rounds + 1):
            for qi, q in enumerate(queries):
                # Rotate condition order, then reverse alternate rounds.
                offset = (qi + round_ - 1) % len(configs)
                order = configs[offset:] + configs[:offset]
                if round_ % 2 == 0:
                    order = list(reversed(order))
                for name, flags in order:
                    run_id = f'{round_}-{q["id"]}-{name}'
                    if run_id in completed:
                        continue
                    directory = args.output / 'artifacts' / run_id
                    extra = []
                    if args.cache != 'off':
                        cache = args.output / 'cache' / (run_id if args.cache == 'cold' else name)
                        extra = ['--cache-ttl', '3600', '--cache-dir', str(cache)]
                        if args.cache == 'warm':
                            capture(binary, q['query'], flags + extra, directory / 'warmup', args.timeout, False)
                    started = dt.datetime.now(dt.timezone.utc).isoformat()
                    run = capture(binary, q['query'], flags + extra, directory, args.timeout, args.trace)
                    run.update(run_id=run_id, query_id=q['id'], query=q['query'], condition=name, round=round_, started_at=started)
                    stream.write(json.dumps(run) + '\n')
                    stream.flush()
                    print(f'{run_id}: {run["process_seconds"]:.3f}s exit={run["exit_code"]} results={len(run["results"])}', flush=True)
    runs = list(map(json.loads, output.read_text().splitlines()))
    summary = {}
    for name, _ in configs:
        rs = [r for r in runs if r['condition'] == name]
        times = sorted(r['process_seconds'] for r in rs)
        summary[name] = dict(runs=len(rs), median_seconds=statistics.median(times),
                             p95_seconds=times[math.ceil(.95 * len(times)) - 1],
                             errors=sum(r['exit_code'] != 0 for r in rs),
                             empty=sum(not r['results'] for r in rs),
                             returned_slots=sum(len(r['results']) for r in rs),
                             expected_slots=5*len(rs))
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
