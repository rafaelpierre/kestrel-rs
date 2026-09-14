#!/usr/bin/env python3
"""macOS peak-RSS comparison using deterministic local pages (issue #26).

Build both revisions with --release --features test-fixtures. No live providers,
ranking, cache or capture: isolates ownership overhead after fetching. Diagnostics
remain enabled. Run sequentially with alternating binary order to limit drift.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import threading


def stable(value):
    if isinstance(value, dict):
        return {k: stable(v) for k, v in value.items()
                if k not in {'elapsed_seconds', 'elapsed_ms', 'timing'}}
    if isinstance(value, list):
        return [stable(v) for v in value]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', required=True, type=Path)
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--trials', type=int, default=5)
    parser.add_argument('--pages', type=int, default=64)
    parser.add_argument('--chars', type=int, default=500000)
    args = parser.parse_args()
    if args.baseline.read_bytes() == args.candidate.read_bytes():
        parser.error('Baseline and candidate binaries are identical; rebuild before measuring')
    args.output.mkdir(parents=True, exist_ok=False)
    phrase = 'Ownership evidence fixture. '
    page = (phrase * (args.chars // len(phrase) + 1))[:args.chars].encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.startswith('/bing'):
                body = ''.join(f'<li class="b_algo"><h2><a href="{endpoint}/page/{i}">Page {i}</a></h2>'
                               '<div class="b_caption"><p>Ownership fixture.</p></div></li>'
                               for i in range(args.pages)).encode()
                kind = 'text/html'
            else:
                body, kind = page, 'text/plain'
            self.send_response(200)
            self.send_header('Content-Type', kind)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    endpoint = f'http://127.0.0.1:{server.server_port}'
    threading.Thread(target=server.serve_forever, daemon=True).start()
    binaries = {k: str(getattr(args, k).resolve()) for k in ('baseline', 'candidate')}
    env = {k: v for k, v in os.environ.items() if not k.startswith('KESTRELSEARCH_')}
    env.update(KESTREL_TEST_PROVIDER_ENDPOINT=endpoint, KESTRELSEARCH_OTEL_ENABLED='false')
    flags = ['search', 'ownership', '-e', 'bing', '--min-results', str(args.pages),
             '--fetch-candidates', str(args.pages), '--content-limit', str(args.chars),
             '--max-response-bytes', str(args.chars + 1000), '--no-rank', '-k', '1',
             '--search-budget', '10', '--fetch-budget', '30', '--timeout', '10', '--output', 'json']
    rows, expected = [], None
    try:
        for trial in range(args.trials):
            order = list(binaries) if trial % 2 == 0 else list(reversed(binaries))
            for name in order:
                argv = ['/usr/bin/time', '-l', binaries[name], *flags]
                result = subprocess.run(argv, env=env, capture_output=True, timeout=60)
                prefix = args.output / f'{name}-{trial + 1}'
                prefix.with_suffix('.stdout').write_bytes(result.stdout)
                prefix.with_suffix('.stderr').write_bytes(result.stderr)
                if result.returncode:
                    raise RuntimeError(result.stderr.decode())
                value = json.loads(result.stdout)
                assert value['diagnostics']['evidence']['extracted'] == args.pages
                assert value['diagnostics']['candidates']['after_selection'] == args.pages
                actual = stable(value)
                if expected is None:
                    expected = actual
                assert actual == expected, 'Output changed beyond timings'
                rss = int(re.search(rb'(\d+)\s+maximum resident set size', result.stderr)[1])
                rows.append({'binary': name, 'trial': trial + 1, 'argv': argv,
                             'exit_code': result.returncode, 'peak_rss_bytes': rss})
    finally:
        server.shutdown()
    report = {'config': {'trials': args.trials, 'pages': args.pages, 'chars': len(page),
                         'ranking': False, 'diagnostics': True, 'capture': False, 'cache': False},
              'binaries': {k: {'path': v, 'sha256': hashlib.sha256(Path(v).read_bytes()).hexdigest()}
                           for k, v in binaries.items()},
              'rows': rows, 'output_equivalent_except_timings': True,
              'median_peak_rss_bytes': {k: statistics.median(r['peak_rss_bytes'] for r in rows if r['binary'] == k)
                                        for k in binaries},
              'limitations': 'macOS fresh-process peak RSS; local plain text, ranking disabled. No live latency or general performance claim.'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['median_peak_rss_bytes']))


if __name__ == '__main__':
    main()
