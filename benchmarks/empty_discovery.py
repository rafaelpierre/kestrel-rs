#!/usr/bin/env python3
"""Matched, bounded discovery policy investigation; raw captures stay local."""
import argparse
from datetime import datetime, timezone
import hashlib
import http.server
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import threading
import time
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[1]
ARMS = ('short', 'long', 'retry_fresh', 'retry_reuse')
ENGINES = ['duckduckgo', 'bing', 'yahoo', 'dogpile', 'ecosia', 'swisscows', 'yep', 'qwant', 'mojeek']
POLICY = dict(short_seconds=1.0, total_wall_seconds=6.0, provider_seconds=5.0,
              backoff_seconds=0.25, max_calls=2, max_sends_per_provider_per_call=3,
              minimum=1, cache='off', rank='off', fetch='off',
              retry='only zero accepted candidates and observed deadline providers; same query',
              percentile='nearest rank ceil(p*n), empty/errors included',
              agent_turn_latency='not measured; orchestration is Python, report separately')


def save(path, value):
    with path.open('x') as f:
        json.dump(value, f, indent=2)
        f.write('\n')


def retryable(o):
    # Do not restart known rate-limited/challenged work, even if backoff hit a deadline.
    attempts = o.get('lifecycle', {}).get('attempts', [])
    blocked = any(a.get('http_status') in (401, 403, 429)
                  or a.get('challenge') == 'detected' or a.get('retry_after')
                  for a in attempts)
    return o['outcome'] == 'deadline' and not blocked


def eligible(results, outcomes, queries):
    # Successful queries are never repeated. Unknown/missing outcomes fail closed.
    accepted = {s['query'] for r in results for s in r.get('sources', [])}
    accepted.update(r.get('query') for r in results)
    return [(q, sorted({o['engine'] for o in outcomes
                       if o['query'] == q and retryable(o)}))
            for q in queries if q not in accepted
            and any(o['query'] == q and retryable(o) for o in outcomes)]


class Probe:
    def __init__(self, binary, directory, endpoint=None):
        self.directory = directory
        directory.mkdir()
        env = os.environ.copy()
        for key in list(env):
            if key.startswith('KESTRELSEARCH_') or key.startswith('KESTREL_TEST_'):
                env.pop(key)
        if endpoint:
            for key in list(env):
                if key.lower().endswith('_proxy'):
                    env.pop(key)
            env['NO_PROXY'] = '127.0.0.1,localhost'
            env['KESTREL_TEST_PROVIDER_ENDPOINT'] = endpoint
        env['KESTRELSEARCH_OTEL_ENABLED'] = 'false'
        env['KESTRELSEARCH_PROVIDER_TRACE_DIR'] = str(directory / 'traces')
        self.err = (directory / 'stderr').open('w')
        self.out = (directory / 'stdout').open('w')
        self.p = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=self.err, text=True, env=env)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.p.stdout, selectors.EVENT_READ)
        self.seen = set()

    def read(self, deadline):
        if not self.selector.select(max(0, deadline - time.monotonic())):
            raise TimeoutError('total wall allowance exhausted')
        line = self.p.stdout.readline()
        self.out.write(line)
        self.out.flush()
        if not line:
            raise RuntimeError('probe exited without response')
        return json.loads(line)

    def call(self, queries, engines, budget, deadline):
        request = dict(queries=queries, engines=engines, budget=budget)
        self.p.stdin.write(json.dumps(request) + '\n')
        self.p.stdin.flush()
        result = self.read(deadline)
        paths = set((self.directory / 'traces').glob('outcome-*.json'))
        outcomes = [json.loads(p.read_text()) for p in sorted(paths - self.seen)]
        self.seen = paths
        return dict(request=request, response=result, outcomes=outcomes)

    def close(self):
        if self.p.poll() is None:
            self.p.stdin.close()
            try:
                self.p.wait(timeout=0.1)
            except subprocess.TimeoutExpired:
                self.p.kill()
                self.p.wait()
        self.selector.close()
        self.p.stdout.close()
        self.err.close()
        self.out.close()
        return self.p.returncode


def measure(binary, directory, arm, query, engines, endpoint=None):
    directory.mkdir()
    start = time.monotonic()
    deadline = start + POLICY['total_wall_seconds']
    row = dict(arm=arm, query=query, started_at=datetime.now(timezone.utc).isoformat(),
               argv=[str(binary)], attempts=[], initializations=[], exit_codes=[])
    probe = None
    try:
        probe = Probe(binary, directory / 'process-1', endpoint)
        row['initializations'].append(probe.read(deadline))
        budget = 1.0 if arm != 'long' else 5.0
        budget = min(budget, max(0.001, deadline - time.monotonic() - 0.2))
        first = probe.call([query], engines, budget, deadline)
        row['attempts'].append(first)
        results = (first['response'].get('report') or {}).get('results', [])
        retry = eligible(results, first['outcomes'], [query])
        if arm.startswith('retry') and retry and first['response']['flushed'] and len(first['outcomes']) == len(engines):
            time.sleep(min(0.25, max(0, deadline - time.monotonic())))
            if arm == 'retry_fresh':
                row['exit_codes'].append(probe.close())
                probe = Probe(binary, directory / 'process-2', endpoint)
                row['initializations'].append(probe.read(deadline))
            remaining = min(3.75, deadline - time.monotonic() - 0.2)
            if remaining > 0:
                q, retry_engines = retry[0]
                row['attempts'].append(probe.call([q], retry_engines, remaining, deadline))
    except (TimeoutError, RuntimeError, BrokenPipeError) as error:
        row['censored_error'] = str(error)
    finally:
        if probe:
            row['exit_codes'].append(probe.close())
    row['ended_at'] = datetime.now(timezone.utc).isoformat()
    row['seconds'] = time.monotonic() - start
    row['results'] = [r for a in row['attempts']
                      for r in (a['response'].get('report') or {}).get('results', [])]
    row['sends'] = sum(o.get('lifecycle', {}).get('send_attempts', 0)
                       for a in row['attempts'] for o in a['outcomes'])
    row['trace_complete'] = all(a['response']['flushed'] and
                               len(a['outcomes']) == len(a['request']['engines'])
                               for a in row['attempts']) and bool(row['attempts'])
    row['discovery_seconds'] = sum(a['response']['seconds'] for a in row['attempts'])
    row['judged_relevance'] = None  # Counts are never relevance judgments.
    save(directory / 'record.json', row)
    return row


class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_GET(self):
        query = parse_qs(urlsplit(self.path).query).get('q', [''])[0]
        with self.server.lock:
            self.server.requests.append(dict(query=query, path=self.path,
                                            connection=self.client_address[1], at=time.monotonic()))
            n = sum(r['query'] == query for r in self.server.requests)
        if query == 'late':
            time.sleep(1.4)
        elif query in ('stalled', 'transient') and (query == 'stalled' or n == 1):
            time.sleep(7)
        status = 403 if query == 'hard' else 429 if query == 'rate' else 200
        body = b'' if query in ('empty', 'hard', 'rate') else (
            '<li class="b_algo"><h2><a href="https://example.org/evidence">'
            'Direct fixture evidence</a></h2><div class="b_caption"><p>'
            f'{query} fixture evidence.</p></div></li>').encode()
        if query == 'empty':
            body = b"<li class='b_no'>No results</li>"
        try:
            self.send_response(status)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            if status == 429:
                self.send_header('Retry-After', '30')
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--live', action='store_true')
    p.add_argument('--rounds', type=int, default=1)
    args = p.parse_args()
    if args.rounds < 1:
        p.error('rounds must be positive')
    binary, root = args.binary.resolve(), args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    from evidence_gate import source_identity
    save(root / 'manifest.json', dict(source_files=source_identity(), policy=POLICY, rounds=args.rounds, live=args.live,
         binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
         revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
         runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()))
    if args.live:
        queries = json.loads((ROOT / 'benchmarks/codex-search-2026-09-11/queries.json').read_text())
    else:
        queries = [dict(id=q, query=q) for q in ('early', 'late', 'transient', 'stalled', 'empty', 'hard', 'rate')]
    rows = []
    for round_ in range(args.rounds):
        for qi, q in enumerate(queries):
            # Matched sequential calls, rotating positions; no concurrent live load.
            offset = (round_ + qi) % len(ARMS)
            for arm in ARMS[offset:] + ARMS[:offset]:
                server = None
                directory = root / f'{round_}-{q["id"]}-{arm}'
                try:
                    if not args.live:
                        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
                        server.daemon_threads = True
                        server.requests, server.lock = [], threading.Lock()
                        threading.Thread(target=server.serve_forever, daemon=True).start()
                    endpoint = f'http://127.0.0.1:{server.server_port}' if server else None
                    row = measure(binary, directory, arm, q['query'], ENGINES if args.live else ['bing'], endpoint)
                    if server:
                        save(directory / 'server.json', server.requests)
                    if server:
                        expected = q['query'] == 'early' or (q['query'] == 'late' and arm != 'short') or (q['query'] == 'transient' and arm.startswith('retry'))
                        assert bool(row['results']) == expected, (q, arm, row)
                        assert row['trace_complete'], (q, arm, 'missing traces')
                        assert row['sends'] == len(server.requests), (q, arm, 'send mismatch')
                        assert row['sends'] <= 6, (q, arm, 'request cap')
                        if q['query'] in ('early', 'empty', 'hard', 'rate'):
                            assert len(row['attempts']) == 1, (q, arm, 'unexpected retry')
                    rows.append(row)
                    print(q['id'], arm, len(row['results']), round(row['seconds'], 3), flush=True)
                finally:
                    if server:
                        server.shutdown()
                        server.server_close()
                time.sleep(0.25)
    summary = {}
    for arm in ARMS:
        cell = [r for r in rows if r['arm'] == arm]
        values = sorted(r['seconds'] for r in cell)
        summary[arm] = dict(n=len(cell), nonempty=sum(bool(r['results']) for r in cell),
                            p50=values[math.ceil(len(values)*.5)-1],
                            p95=values[math.ceil(len(values)*.95)-1],
                            sends=sum(r['sends'] for r in cell),
                            missing_traces=sum(not r['trace_complete'] for r in cell))
    save(root / 'summary.json', summary)
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
