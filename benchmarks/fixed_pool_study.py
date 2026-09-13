#!/usr/bin/env python3
"""Version 2 candidate-cap study: freeze live metadata before varying fetch caps."""
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from urllib.parse import urlsplit, urlunsplit, parse_qsl, urlencode

from quality_conditions import conditions
from quality_latency import capture, environment, schedule, sha

ROOT = Path(__file__).resolve().parents[1]


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write('\n')


def canonical(value):
    """Mirror search.rs tracking/fragment/trailing-slash normalization for pool union."""
    url = urlsplit(value)
    if url.scheme not in {"http", "https"} or not url.hostname:
        raise ValueError("pool URL must be HTTP(S)")
    host = url.hostname.lower()
    if ":" in host:
        host = "[" + host + "]"
    if url.port and (url.scheme, url.port) not in {("http", 80), ("https", 443)}:
        host += ":" + str(url.port)
    query = [(k, v) for k, v in parse_qsl(url.query, keep_blank_values=True)
             if not k.lower().startswith("utm_") and k.lower() not in {"fbclid", "gclid", "msclkid"}]
    return urlunsplit((url.scheme, host, url.path.rstrip("/") or "/", urlencode(query), ""))


def freeze_pool(query, captures):
    """First occurrence wins, preserving discovery order and complete metadata."""
    rows, seen = [], set()
    for capture_ in captures:
        if capture_['exit_code'] != 0:
            continue
        for row in capture_['results']:
            key = canonical(row['url'])
            if key not in seen:
                seen.add(key)
                rows.append(row)
    return {'queries': [query], 'candidates': rows, 'sufficient': len(rows) >= 15}


def execute(argv, directory):
    directory.mkdir()
    start = dt.datetime.now(dt.timezone.utc).isoformat()
    clock = time.monotonic()
    env = os.environ.copy()
    env['KESTRELSEARCH_OTEL_ENABLED'] = 'false'
    try:
        proc = subprocess.run(argv, capture_output=True, timeout=30, env=env)
        stdout, stderr, status = proc.stdout, proc.stderr, proc.returncode
    except subprocess.TimeoutExpired as error:
        stdout, stderr, status = error.stdout or b'', error.stderr or b'', 124
    seconds = time.monotonic() - clock
    (directory / 'stdout').write_bytes(stdout)
    (directory / 'stderr').write_bytes(stderr)
    record = dict(argv=argv, started_at=start, process_seconds=seconds, exit_code=status)
    save(directory / 'receipt.json', record)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--replay', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = output / 'kestrel'
    replay = output / 'fetch_cap_replay'
    shutil.copy2(args.binary, binary)
    shutil.copy2(args.replay, replay)
    dataset = ROOT / 'benchmarks/quality-queries-v1.json'
    queries = json.loads(dataset.read_text())['queries']
    policy = conditions('retrieval', minimum=30, top_k=30, search_budget=10)[-1]
    # Explicitly freeze one policy, rather than silently using a sweep arm's minimum.
    from quality_conditions import flags_for
    policy['effective']['minimum'] = 30
    policy['flags'] = flags_for(policy['effective'])
    sources = ['Cargo.toml', 'Cargo.lock', 'examples/fetch_cap_replay.rs',
               'benchmarks/fixed_pool_study.py', 'benchmarks/quality_conditions.py',
               'benchmarks/quality_latency.py', 'benchmarks/report_fixed_pool.py']
    sources += subprocess.check_output(['git', 'ls-files', 'src'], cwd=ROOT, text=True).splitlines()
    save(output / 'manifest.json', dict(
        schema_version=2, revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        tree=subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=ROOT, text=True).strip(),
        diff=subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT, text=True),
        source_hashes={p: sha(ROOT / p) for p in sources},
        binary_sha256=sha(binary), replay_sha256=sha(replay), dataset_sha256=sha(dataset),
        version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
        environment=environment(), load_average=os.getloadavg(), discovery=policy, rounds=3, caps=[5, 10, 15], pace_seconds=0.25,
        pool_policy='Exactly two same-query discoveries; stable canonical URL union, no padding or recovery. All 14 queries retained, insufficient pools skipped.',
        fetch_policy='Fresh process/client, cache off, hybrid rank, top 5, pre-rank off, score gate off; 5s fetch budget, 10s request timeout, concurrency/parse 10, content 2000, bytes 1000000; PDF URLs skipped as in CLI.',
        inference='Conditional live-fetch cap comparison; discovery excluded; changing bodies/upstream cache uncontrolled. Pool union is not a single search output.',
        assessor='Codex GPT-6; unblinded semantic relevance 0/1/2 and evidence 0/1; unknown judgments stay unknown',
        statistics='nearest-rank p50/p95; three repeats per query are exploratory, no reliable population p95 or causal total-search estimate'))
    eligible = []
    for query in queries:
        directory = output / query['id']
        directory.mkdir()
        captures = []
        for attempt in (1, 2):
            start = dt.datetime.now(dt.timezone.utc).isoformat()
            os.environ['KESTRELSEARCH_OTEL_ENABLED'] = 'false'
            result = capture(binary, query['query'], policy['flags'], directory / f'discovery-{attempt}', 30, True)
            result['started_at'] = start
            save(directory / f'discovery-{attempt}.json', result)
            captures.append(result)
            time.sleep(0.25)
        pool = freeze_pool(query['query'], captures)
        save(directory / 'pool.json', pool)
        print(query['id'], len(pool['candidates']), flush=True)
        if pool['sufficient']:
            eligible.append(query)
    for round_, query, cap in schedule([5, 10, 15], eligible, 3):
        directory = output / query['id']
        pool = directory / 'pool.json'
        record = execute([str(replay), str(pool), str(cap)], directory / f'round-{round_}-cap-{cap}')
        print(query['id'], round_, cap, record['exit_code'], flush=True)
        time.sleep(0.25)


if __name__ == '__main__':
    main()
