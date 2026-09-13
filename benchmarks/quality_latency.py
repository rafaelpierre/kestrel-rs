#!/usr/bin/env python3
"""Sequential, resumable current-CLI ablations; historical batches remain immutable."""
import argparse
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time

from quality_conditions import ENGINES, STAGES, conditions
from quality_report import judgment_template, load_judgments, pool_check, summarize

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_QUERIES = ROOT / 'benchmarks/quality-queries-v1.json'


def configurations(stage, engine=None):
    """Compatibility helper for consumers inspecting named CLI arguments."""
    return [(c['name'], c['flags']) for c in conditions(stage, engine)]


def capture(binary, query, args, directory, timeout, trace):
    directory.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    # Never leak inherited trace destinations into another batch.
    env.pop('KESTRELSEARCH_PROVIDER_TRACE_DIR', None)
    env['KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR'] = str(directory)
    env['KESTRELSEARCH_BENCHMARK_RUN_ID'] = 'run'
    if trace:
        env['KESTRELSEARCH_PROVIDER_TRACE_DIR'] = str(directory / 'providers')
    command = [str(binary), 'search', query, *args]
    if '--top-k' not in args and '-k' not in args:
        command += ['--top-k', '5']
    command += ['--output', 'json']
    start = time.perf_counter()
    try:
        proc = subprocess.run(command, capture_output=True, text=True, timeout=timeout, env=env)
        stdout, stderr, status = proc.stdout, proc.stderr, proc.returncode
    except subprocess.TimeoutExpired as error:
        def text(v):
            return v.decode(errors='replace') if isinstance(v, bytes) else v or ''
        stdout, stderr, status = text(error.stdout), text(error.stderr), 124
    elapsed = time.perf_counter() - start
    command_seconds = None
    try:
        payload = json.loads(stdout)
        results = payload['results'] if isinstance(payload, dict) else payload
        if not isinstance(results, list) or any(not isinstance(r, dict) or not isinstance(r.get('url'), str) for r in results):
            raise ValueError('expected result records')
        if isinstance(payload, dict):
            seconds = payload.get('elapsed_seconds')
            if isinstance(seconds, (int, float)) and not isinstance(seconds, bool) and math.isfinite(seconds) and seconds >= 0:
                command_seconds = seconds
    except (ValueError, TypeError, KeyError):
        results = []
        if status == 0:
            status = 65
    artifacts = []
    artifact_errors = []
    for p in sorted(directory.glob('run-*.json')):
        try:
            artifacts.append(json.loads(p.read_text()))
        except (ValueError, OSError) as error:
            artifact_errors.append(f'{p.name}: {error}')
    return dict(command=command, process_seconds=elapsed, command_seconds=command_seconds,
                exit_code=status, stdout=stdout, stderr=stderr, results=results, artifacts=artifacts,
                artifact_errors=artifact_errors)


def positive(value):
    value = float(value)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError('must be finite and positive')
    return value


def schedule(configs, queries, rounds):
    # Full cycles balance position per query; reversal balances directional drift.
    for round_ in range(1, rounds + 1):
        for qi, q in enumerate(queries):
            offset = (qi + round_ - 1) % len(configs)
            order = configs[offset:] + configs[:offset]
            if (round_ - 1) // len(configs) % 2:
                order = list(reversed(order))
            for condition in order:
                yield round_, q, condition


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def environment():
    return dict(platform=platform.platform(), machine=platform.machine(), python=sys.version,
                cpu_count=os.cpu_count(), process_concurrency=1,
                locale={k: os.environ.get(k) for k in ['LANG', 'LC_ALL', 'TZ']},
                proxy_variables_present=[k for k in ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
                                                   'http_proxy', 'https_proxy', 'all_proxy', 'no_proxy'] if k in os.environ],
                upstream_cache='uncontrolled', client_reuse='new CLI process each call',
                warm_state='page cache only; warmup success does not guarantee the next live search chooses the same URLs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--queries', type=Path, default=DEFAULT_QUERIES)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--stage', choices=STAGES, default='retrieval')
    parser.add_argument('--engine', choices=ENGINES,
                        help='Use one engine instead of the explicit nine-engine list')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--timeout', type=positive, default=50.0)
    parser.add_argument('--pace', type=positive, default=0.25, help='Minimum idle seconds between subprocess calls')
    parser.add_argument('--trace', action='store_true')
    parser.add_argument('--search-budget', type=positive)
    parser.add_argument('--fetch-budget', type=positive)
    parser.add_argument('--min-results', type=int, default=15, help='Fixed collection target outside the minimum sweep')
    parser.add_argument('--candidate-pool', type=int, default=15)
    parser.add_argument('--top-k', type=int, default=5)
    parser.add_argument('--query-syntax', choices=['portable', 'native'], default='portable')
    parser.add_argument('--limit', type=int)
    parser.add_argument('--query-id', action='append', help='Select versioned queries by ID, repeatable')
    parser.add_argument('--resume', action='store_true')
    parser.add_argument('--cache', choices=['off', 'cold', 'warm'], default='off')
    parser.add_argument('--dry-run', action='store_true', help='Emit effective policies and exact commands without searching')
    parser.add_argument('--judgments', type=Path, help='Versioned query/URL/content judgments to include in summaries')
    args = parser.parse_args()
    if args.rounds < 1 or (args.limit is not None and args.limit < 1):
        parser.error('rounds and limit must be positive')
    try:
        configs = conditions(args.stage, args.engine, top_k=args.top_k, minimum=args.min_results,
                             pool=args.candidate_pool, search_budget=args.search_budget,
                             fetch_budget=args.fetch_budget, syntax=args.query_syntax, cache=args.cache)
        judgments = load_judgments(args.judgments)
    except ValueError as error:
        parser.error(str(error))
    binary = args.binary.resolve(strict=True)
    raw = json.loads(args.queries.read_text())
    queries = raw if isinstance(raw, list) else raw['queries']
    ids = [q['id'] for q in queries]
    if any(not isinstance(i, str) or not re.fullmatch(r'[A-Za-z0-9_-]+', i) for i in ids):
        parser.error('query IDs must contain only letters, digits, underscores and hyphens')
    if len(set(ids)) != len(ids) or any(not isinstance(q.get('query'), str) or not q['query'].strip() for q in queries):
        parser.error('queries require unique IDs and nonempty query text')
    if args.query_id:
        if set(args.query_id) - set(ids):
            parser.error('unknown query ID')
        queries = [q for q in queries if q['id'] in args.query_id]
    queries = queries[:args.limit]
    if not queries:
        parser.error('empty query selection')
    if args.dry_run:
        print(json.dumps(dict(schema_version=2, conditions=configs,
            schedule=[dict(round=r, query_id=q['id'], condition=c['name'],
                           command=[str(binary), 'search', q['query'], *c['flags'], '--output', 'json'])
                      for r, q, c in schedule(configs, queries, args.rounds)]), indent=2))
        return
    if args.output is None:
        parser.error('--output is required except with --dry-run')
    if args.output.exists() and not args.resume:
        parser.error('output exists; choose a new directory or --resume')
    if args.resume and not (args.output / 'metadata.json').exists():
        parser.error('resume requires an existing versioned batch')
    args.output = args.output.resolve()
    paths = [*sorted((ROOT / 'src').rglob('*.rs')), ROOT / 'Cargo.toml', ROOT / 'Cargo.lock',
             Path(__file__), ROOT / 'benchmarks/quality_conditions.py', ROOT / 'benchmarks/quality_report.py',
             ROOT / 'benchmarks/run_quality_study.py', ROOT / 'benchmarks/replay_ranking.py']
    metadata = dict(schema_version=2, source_sha256={str(p.relative_to(ROOT)): sha(p) for p in paths},
                    binary=str(binary), binary_sha256=sha(binary),
                    version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
                    commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    tracked_diff_sha256=hashlib.sha256(subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT)).hexdigest(),
                    query_sha256=sha(args.queries), query_file=str(args.queries.resolve()),
                    stage=args.stage, rounds=args.rounds, cache=args.cache, conditions=configs,
                    environment=environment(), pace_seconds=args.pace, timeout=args.timeout, trace=args.trace,
                    judgment_sha256=sha(args.judgments) if args.judgments else None,
                    query_ids=[q['id'] for q in queries],
                    order='rotating condition position per query; reversed alternate full cycles',
                    balance_complete=args.rounds % len(configs) == 0,
                    timing='process includes startup and timeout waits; command uses CLI elapsed_seconds; timeouts are censored')
    meta_path = args.output / 'metadata.json'
    if args.resume and json.loads(meta_path.read_text()) != json.loads(json.dumps(metadata)):
        parser.error('resume metadata differs; choose a new output directory')
    args.output.mkdir(parents=True, exist_ok=True)
    if not args.resume:
        meta_path.write_text(json.dumps(metadata, indent=2) + '\n')
        (args.output / 'queries.json').write_text(json.dumps(queries, indent=2) + '\n')
    frozen = args.output / '.binary' / 'kestrel'
    frozen.parent.mkdir(exist_ok=True)
    if not frozen.exists():
        shutil.copy2(binary, frozen)
    if sha(frozen) != metadata['binary_sha256']:
        parser.error('frozen binary differs; use a new batch')
    output = args.output / 'runs.jsonl'
    old = list(map(json.loads, output.read_text().splitlines())) if output.exists() else []
    completed = {r['run_id'] for r in old}
    if len(completed) != len(old):
        parser.error('duplicate run IDs in saved batch')
    with output.open('a') as stream:
        for round_, q, condition in schedule(configs, queries, args.rounds):
            name, flags, policy = condition['name'], condition['flags'], condition['effective']
            run_id = f'{round_}-{q["id"]}-{name}'
            if run_id in completed:
                continue
            # Preserve incomplete attempts after interruption; never mix their artifacts.
            root = args.output / 'artifacts' / run_id
            root.mkdir(parents=True, exist_ok=True)
            directory = root / f'attempt-{len(list(root.iterdir())) + 1}'
            directory.mkdir()
            extra = []
            warmup = None
            if args.cache != 'off':
                cache = directory / 'cache'
                extra = ['--cache-ttl', '3600', '--cache-dir', str(cache)]
                if args.cache == 'warm':
                    time.sleep(args.pace)
                    warmup = capture(frozen, q['query'], flags + extra, directory / 'warmup', args.timeout, False)
                    (directory / 'warmup.json').write_text(json.dumps(warmup) + '\n')
            time.sleep(args.pace)
            started = dt.datetime.now(dt.timezone.utc).isoformat()
            run = capture(frozen, q['query'], flags + extra, directory, args.timeout, args.trace)
            run.update(run_id=run_id, query_id=q['id'], query=q['query'], condition=name,
                       effective=policy, round=round_, started_at=started,
                       warmup_exit_code=warmup['exit_code'] if warmup else None,
                       artifact_directory=str(directory))
            run['pool_check'] = pool_check(run, args.candidate_pool)
            stream.write(json.dumps(run) + '\n')
            stream.flush()
            print(f'{run_id}: {run["process_seconds"]:.3f}s exit={run["exit_code"]} results={len(run["results"])}', flush=True)
    runs = list(map(json.loads, output.read_text().splitlines()))
    (args.output / 'summary.json').write_text(json.dumps(summarize(runs, judgments), indent=2) + '\n')
    (args.output / 'judgments-template.json').write_text(json.dumps(judgment_template(runs), indent=2) + '\n')
    print(f'Saved {len(runs)} runs to {args.output}')


if __name__ == '__main__':
    main()
