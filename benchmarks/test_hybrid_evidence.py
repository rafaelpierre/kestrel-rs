"""Opt-in deterministic checks for the benchmark's isolation and small pools."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.environ.get('KESTREL_HYBRID_REPLAY'), 'set KESTREL_HYBRID_REPLAY')
class ReplayTests(unittest.TestCase):
    def run_pool(self, pool, mode='replay'):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'pool.json'
            path.write_text(json.dumps(pool))
            return subprocess.run([os.environ['KESTREL_HYBRID_REPLAY'], str(path), mode, 'provider'],
                                  capture_output=True, text=True, timeout=10)

    def test_empty_and_small_multi_query_preserve_provenance(self):
        for size in (0, 1, 2, 3):
            rows = [dict(title='rust ownership', url=f'https://example.test/{i}',
                         display_url='', snippet='borrowing', content=None, bm25_score=None,
                         query=('rust' if i % 2 else 'ownership'), engine=None,
                         engine_rank=None, sources=[]) for i in range(size)]
            rows = [{k: v for k, v in r.items() if k not in ('engine', 'engine_rank', 'bm25_score', 'sources')} for r in rows]
            result = self.run_pool(dict(queries=['ownership', 'rust'], candidates=rows))
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(result.stdout)
            self.assertEqual(data['candidates'], rows)
            for ordering in data['orderings']:
                if ordering['policy'] != 'Body':
                    self.assertEqual(len(ordering['results']), size)
                    self.assertTrue(all(r in rows for r in ordering['results']))
            self.assertIsNone(data['fetch_report'])

    def test_live_arm_rejects_preexisting_body(self):
        row = dict(title='rust', url='https://example.test', display_url='', snippet='',
                   content='retained body', bm25_score=None, query='rust', engine=None,
                   engine_rank=None, sources=[])
        result = self.run_pool(dict(queries=['rust'], candidates=[row]), 'metadata')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unfetched', result.stderr)
