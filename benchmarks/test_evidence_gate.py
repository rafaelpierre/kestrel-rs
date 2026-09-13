"""Deterministic provenance and budget regressions; no live providers."""
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import evidence_gate as gate


class EvidenceGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.run = Path(self.temp.name)
        gate.save(self.run / 'queries.json', [{'id': f'q{i:02d}', 'query': 'Australia capital', 'intent': 'Canberra'} for i in range(1, 11)])

    def evidence(self):
        attempt = self.run / 'q01/search-1'
        attempt.mkdir(parents=True)
        (attempt / 'stdout').write_text(json.dumps({'results': [{'url': 'https://example.org', 'snippet': 'Canberra is the capital.'}]}))
        gate.save(attempt / 'receipt.json', {'exit_code': 0, 'stdout_sha256': gate.sha(attempt / 'stdout'), 'seconds': 0.5})
        return attempt

    def judgment(self):
        return {'verdict': 'pass', 'layer': 'none', 'answer': 'Canberra.', 'rationale': 'Explicit statement',
                'inspection': 'Inspected all candidates', 'fetch_disposition': 'Skipped: snippet sufficient',
                'citations': [{'attempt': 'search-1', 'url': 'https://example.org', 'passage': 'Canberra is the capital.'}]}

    def test_reject_fabricated_passage_and_wrong_url(self):
        self.evidence()
        judgment = self.judgment()
        gate.validate_assessment(self.run, 'q01', judgment)
        for key, value in [('passage', 'Sydney is the capital.'), ('url', 'https://other.org')]:
            broken = self.judgment()
            broken['citations'][0][key] = value
            with self.assertRaises(ValueError):
                gate.validate_assessment(self.run, 'q01', broken)

    def test_capture_timeout_keeps_partial_streams(self):
        attempt = self.run / 'timeout'
        def interrupted(argv, **kwargs):
            kwargs['stdout'].write(b'partial')
            kwargs['stderr'].write(b'diagnostic')
            raise subprocess.TimeoutExpired(argv, kwargs['timeout'])
        with patch.object(gate.subprocess, 'run', side_effect=interrupted):
            result = gate.execute(attempt, [sys.executable, '-V'], timeout=30)
        self.assertTrue(result['timed_out'])
        self.assertEqual(result['exit_code'], 124)
        self.assertIn('partial', (attempt / 'stdout').read_text())
        self.assertIn('diagnostic', (attempt / 'stderr').read_text())
        with self.assertRaises(FileExistsError):
            gate.execute(attempt, [sys.executable, '-V'])

    def test_budget_counts_interrupted_attempt_and_ignores_decision_files(self):
        self.evidence()
        (self.run / 'q01/search-2').mkdir()
        (self.run / 'q01/search-1-decision.json').write_text('{}')
        self.assertEqual(len(gate.attempts(self.run, 'q01', 'search')), 2)
        with patch.object(gate, 'check', return_value={}), self.assertRaisesRegex(ValueError, 'budget exhausted'):
            gate.retrieve(self.run, 'q01', 'search', 'Australia capital', 'Retry')

    def test_reject_undiscovered_fetch_and_initial_rewrite(self):
        with patch.object(gate, 'check', return_value={}):
            with self.assertRaisesRegex(ValueError, 'URL returned'):
                gate.retrieve(self.run, 'q01', 'fetch', 'https://remembered.org', 'Known URL')
            with self.assertRaisesRegex(ValueError, 'exact manifest'):
                gate.retrieve(self.run, 'q01', 'search', 'rewritten')

    def test_missing_assessment_and_incomplete_attempt_never_pass(self):
        self.evidence()
        gate.save(self.run / 'manifest.json', {'dataset_sha256': gate.sha(self.run / 'queries.json')})
        self.assertIn('NOT PASSED', gate.report(self.run)['gate'])
        gate.save(self.run / 'q01/assessment.json', self.judgment())
        (self.run / 'q01/fetch-1').mkdir()
        report = gate.report(self.run)
        self.assertIn('NOT PASSED', report['gate'])
        self.assertIsNone(report['questions'][0]['total_tool_seconds'])

    def test_mutated_evidence_rejected(self):
        attempt = self.evidence()
        (attempt / 'stdout').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'changed'):
            gate.validate_assessment(self.run, 'q01', self.judgment())

    def test_interruption_before_request_still_reserves_attempt(self):
        with patch.object(gate, 'check', return_value={'binary': sys.executable}), patch.object(gate, 'save', side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                gate.retrieve(self.run, 'q01', 'search')
        self.assertEqual(len(gate.attempts(self.run, 'q01', 'search')), 1)
        self.assertFalse((self.run / 'q01/search-1/receipt.json').exists())

    def test_reject_missing_duplicate_and_extra_dataset_questions(self):
        valid = gate.read(self.run / 'queries.json')
        for invalid in (valid[:-1], valid + [valid[-1]], [valid[0]] * 10):
            (self.run / 'queries.json').write_text(json.dumps(invalid))
            with self.assertRaisesRegex(ValueError, 'exactly q01-q10'):
                gate.canonical_questions(self.run / 'queries.json')

    def test_recovery_preserves_site(self):
        (self.run / 'queries.json').unlink()
        gate.save(self.run / 'queries.json', [{'id': 'q01', 'query': 'site:example.org topic',
                                             'domain': 'example.org', 'intent': 'topic'}])
        self.evidence()
        with patch.object(gate, 'check', return_value={}), self.assertRaisesRegex(ValueError, 'site restriction'):
            gate.retrieve(self.run, 'q01', 'search', 'topic', 'Missing official source')


if __name__ == '__main__':
    unittest.main()
