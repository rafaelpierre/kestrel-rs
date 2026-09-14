"""Deterministic report checks: policy replay must not multiply arm costs."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class ReportTests(unittest.TestCase):
    def test_costs_once_per_arm_with_four_policies_and_blocked_query(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def save(path, data):
                path = root / path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(json.dumps(data))

            ids = [f'q{i:02}' for i in range(8)]
            save('manifest.json', {'queries': [{'id': q} for q in ids + ['blocked']]})
            save('blocked/blocked.json', {'reason': 'no candidates'})
            labels = []
            for q in ids:
                candidates = [{'url': f'https://example.test/{i}', 'content': 'evidence' if i < 5 else None}
                              for i in range(15)]
                for r in candidates:
                    labels.append(dict(query_id=q, url=r['url'], relevance=2,
                                       evidence=int(bool(r['content'])),
                                       content_sha256=hashlib.sha256((r['content'] or '').encode()).hexdigest()))
                for mode in ('fetch', 'metadata'):
                    rows = candidates if mode == 'fetch' else [{**r, 'content': None} for r in candidates]
                    data = dict(selected_urls=[r['url'] for r in rows], candidates=rows,
                                orderings=[dict(policy=p, results=rows) for p in ('Provider', 'Snippet', 'Body', 'Hybrid')],
                                fetch_seconds=2 if mode == 'fetch' else 0,
                                fetch_report={'pages': [{'response_bytes': 100} for _ in rows]} if mode == 'fetch' else None)
                    for round_ in (1, 2):
                        save(f'{q}/{round_}-{mode}/stdout', data)
                        save(f'{q}/{round_}-{mode}/receipt.json', {'seconds': 3 if mode == 'fetch' else 1})
                    if mode == 'fetch':
                        save(f'{q}/selection-pre-rank/stdout', data)
            save('judgments.json', {'judgments': labels})
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('report_hybrid_evidence.py')),
                                     str(root), str(root / 'judgments.json')], capture_output=True, text=True, check=True)
            report = json.loads(result.stdout)
            self.assertEqual(report['schema_version'], 2)
            arms = report['arms']
            self.assertEqual(len(arms), 32)
            self.assertEqual(len({(a['query_id'], a['round'], a['mode']) for a in arms}), 32)
            expected = dict(page_attempts=240, response_bytes=24000, extracted=80,
                            process_seconds=64, fetch_seconds=32)
            for field, total in expected.items():
                self.assertEqual(sum(a[field] for a in arms), total, field)
                self.assertTrue(all(field not in row for row in report['rows']), field)
            metadata = [a for a in arms if a['mode'] == 'metadata']
            self.assertTrue(all(a['page_attempts'] == a['response_bytes'] == a['extracted'] == a['fetch_seconds'] == 0 for a in metadata))
            rows = [r for r in report['rows'] if 'blocked' not in r]
            self.assertEqual(len(rows), 128)
            self.assertTrue(all(r['precision_at_5'] == 1 for r in rows))
            self.assertTrue(all(r['evidence_at_5'] == (1 if r['mode'] == 'fetch' else 0) for r in rows))
            self.assertEqual(len(report['separate_pre_rank']), 8)
            self.assertEqual(report['rows'][-1], {'query_id': 'blocked', 'blocked': {'reason': 'no candidates'}})
