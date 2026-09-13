"""Deterministic frozen-pool and real fetch/rank replay regressions."""
import collections
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

from fixed_pool_study import freeze_pool, canonical
from report_fixed_pool import summarize, percentile


class PoolTests(unittest.TestCase):
    def test_union_preserves_order_metadata_and_ignores_failed_discoveries(self):
        first = dict(url='https://example.org/a', title='first')
        second = dict(url='https://example.org/b', title='second')
        pool = freeze_pool('q', [dict(exit_code=0, results=[first]),
                               dict(exit_code=1, results=[second]),
                               dict(exit_code=0, results=[dict(first, title='changed'), second])])
        self.assertEqual(pool['candidates'], [first, second])
        self.assertFalse(pool['sufficient'])

    def test_tracking_variants_do_not_inflate_pool(self):
        self.assertEqual(canonical('https://EXAMPLE.org:443/a/?utm_source=x&v=1#top'),
                         'https://example.org/a?v=1')
        rows = [dict(url='https://example.org/a?utm_source=' + str(i)) for i in range(15)]
        self.assertEqual(len(freeze_pool('q', [dict(exit_code=0, results=rows)])['candidates']), 1)

    def test_pool_minimum_is_measured_not_assumed(self):
        rows = [dict(url=f'https://example.org/{i}') for i in range(15)]
        self.assertTrue(freeze_pool('q', [dict(exit_code=0, results=rows)])['sufficient'])
        self.assertFalse(freeze_pool('q', [dict(exit_code=0, results=rows[:14])])['sufficient'])

    def test_report_retains_insufficient_pool_without_inventing_measurements(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'manifest.json').write_text('{}')
            (root / 'q01').mkdir()
            (root / 'q01/pool.json').write_text(json.dumps(dict(candidates=[], sufficient=False)))
            result = summarize(root)
            self.assertFalse(result['pools'][0]['sufficient'])
            self.assertEqual(result['measurements'], [])
            for row in result['summaries']:
                self.assertEqual(row['n'], 0)
                self.assertIsNone(row['command_seconds']['p50'])
                self.assertIsNone(row['judged_relevance'])
        self.assertEqual(percentile([3, 1, 2], .95), 3)
        self.assertIsNone(percentile([], .5))

    @unittest.skipUnless(os.environ.get('KESTREL_CAP_REPLAY_BINARY'), 'set KESTREL_CAP_REPLAY_BINARY')
    def test_live_local_pages_exercise_prefix_caps_and_reject_bad_pools(self):
        requests = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.path)
                body = b'<html><main><p>Canberra is the capital of Australia.</p></main></html>'
                self.send_response(200)
                self.send_header('Content-Type', 'text/html')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'pool.json'
                rows = [dict(url=f'http://127.0.0.1:{server.server_port}/{i}', title='Canberra',
                             snippet='capital Australia', display_url='local', content=None)
                        for i in range(15)]
                env = dict(os.environ, NO_PROXY='127.0.0.1', no_proxy='127.0.0.1', KESTRELSEARCH_OTEL_ENABLED='false')
                def run(pool, cap):
                    path.write_text(json.dumps(dict(queries=['capital Australia'], candidates=pool)))
                    return subprocess.run([os.environ['KESTREL_CAP_REPLAY_BINARY'], str(path), str(cap)],
                                          capture_output=True, text=True, timeout=15, env=env)
                for cap in (5, 10, 15):
                    requests.clear()
                    result = run(rows, cap)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    data = json.loads(result.stdout)
                    self.assertEqual(collections.Counter(requests), collections.Counter(f'/{i}' for i in range(cap)))
                    self.assertEqual(len(data['results']), 5)
                    self.assertTrue(all('Canberra' in r['content'] for r in data['results']))
                    self.assertEqual(data['selected_urls'], [r['url'] for r in rows[:cap]])
                requests.clear()
                for pool, cap in [(rows[:14], 5), (rows + [rows[0]], 5), (rows, 0),
                                  ([dict(rows[0], content='stale')] + rows[1:], 5)]:
                    self.assertNotEqual(run(pool, cap).returncode, 0)
                self.assertEqual(requests, [])
                pdf_rows = [dict(rows[0], url=rows[0]['url'] + '.pdf')] + rows[1:]
                result = run(pdf_rows, 5)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(requests), 4)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == '__main__':
    unittest.main()
