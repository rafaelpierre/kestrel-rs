"""Offline regression coverage for current-CLI experiment contracts."""
import copy
import json
import os
import sys
import contextlib
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import quality_conditions as qc
import quality_latency as ql
import quality_report as qr


class ConditionsTests(unittest.TestCase):
    def test_supported_distinct_policies(self):
        for stage in qc.STAGES:
            items = qc.conditions(stage)
            qc.check_duplicates(items)
            for item in items:
                self.assertNotIn('--provider-quorum', item['flags'])
                self.assertEqual(item['flags'].count('--engine'), len(item['effective']['engines']))
        self.assertEqual([c['effective']['minimum'] for c in qc.conditions('retrieval')], [1, 5, 15])
        budgets = qc.conditions('search-budget')
        self.assertEqual([c['effective']['search_budget_seconds'] for c in budgets], [1, 2, 3, 5, None])
        self.assertNotIn('--search-budget', budgets[-2]['flags'])
        self.assertIn('--no-search-budget', budgets[-1]['flags'])

    def test_duplicate_effective_default_and_explicit_deadline_rejected(self):
        original = qc.conditions('search-budget')[-2]
        duplicate = copy.deepcopy(original)
        duplicate['name'] = 'explicit-five'
        duplicate['effective']['search_budget_mode'] = 'explicit'
        with self.assertRaisesRegex(ValueError, 'duplicate effective'):
            qc.check_duplicates([original, duplicate])

    def test_invalid_stage_overrides_and_metadata_cache_rejected(self):
        for stage in ['retrieval', 'providers', 'search-budget', 'fetching', 'swisscows']:
            for overrides in [dict(cache='cold'), dict(cache='warm'), dict(fetch_budget=1)]:
                with self.subTest(stage=stage, overrides=overrides), self.assertRaises(ValueError):
                    qc.conditions(stage, **overrides)
        for stage, kw in [('search-budget', dict(search_budget=1)), ('fetch-budget', dict(fetch_budget=1)),
                          ('candidates', dict(minimum=5)), ('candidates', dict(pool=5))]:
            with self.assertRaises(ValueError):
                qc.conditions(stage, **kw)

    def test_candidate_sweep_changes_only_cap_with_sufficient_target(self):
        items = qc.conditions('candidates')
        policies = [{k: v for k, v in c['effective'].items() if k != 'candidate_cap'} for c in items]
        self.assertEqual(policies[0], policies[1])
        self.assertEqual(policies[1], policies[2])
        self.assertEqual([c['effective']['candidate_cap'] for c in items], [5, 10, 15])
        self.assertGreaterEqual(policies[0]['minimum'], 15)

    def test_full_cycle_counterbalances_positions_per_query(self):
        configs = qc.conditions('retrieval')
        queries = [dict(id='a'), dict(id='b')]
        positions = {q['id']: {c['name']: [] for c in configs} for q in queries}
        for r in range(6):
            block = list(ql.schedule(configs, queries, 6))[r*6:(r+1)*6]
            for i, (_, query, condition) in enumerate(block):
                positions[query['id']][condition['name']].append(i % 3)
        for by_condition in positions.values():
            for found in by_condition.values():
                self.assertEqual(sorted(found), [0, 0, 1, 1, 2, 2])

    @unittest.skipUnless(os.environ.get('KESTREL_BENCH_TEST_BINARY'), 'set KESTREL_BENCH_TEST_BINARY for real CLI parsing')
    def test_every_generated_condition_parses_in_current_cli_without_network(self):
        # An unterminated portable phrase fails query validation before provider requests.
        for stage in qc.STAGES:
            for c in qc.conditions(stage):
                p = subprocess.run([os.environ['KESTREL_BENCH_TEST_BINARY'], 'search', '"', *c['flags'], '--output', 'json'],
                                   capture_output=True, text=True, timeout=10)
                self.assertEqual(p.returncode, 1, (stage, c['name'], p.stderr))
                self.assertIn('unclosed quoted phrase', p.stderr.lower())


class CaptureTests(unittest.TestCase):
    def capture(self, payload, status=0):
        with tempfile.TemporaryDirectory() as d, patch('subprocess.run', return_value=subprocess.CompletedProcess([], status, json.dumps(payload), '')):
            return ql.capture(Path('/fixture'), 'q', [], Path(d), 1, False)

    def test_command_time_is_separate_and_legacy_is_unknown(self):
        self.assertEqual(self.capture(dict(results=[], elapsed_seconds=.125))['command_seconds'], .125)
        self.assertIsNone(self.capture([])['command_seconds'])
        self.assertIsNone(self.capture(dict(results=[], elapsed_seconds=-1))['command_seconds'])

    def test_malformed_envelopes_are_errors_not_valid_empty(self):
        for bad in [{}, {'results': {}}, {'results': [None]}, {'results': [{'title': 'no URL'}]}]:
            self.assertEqual(self.capture(bad)['exit_code'], 65)

    def test_timeout_preserved_and_trace_destination_isolated(self):
        with tempfile.TemporaryDirectory() as d, patch.dict(os.environ, {'KESTRELSEARCH_PROVIDER_TRACE_DIR': '/unrelated'}):
            with patch('subprocess.run', side_effect=subprocess.TimeoutExpired([], 1, b'', b'timed out')) as mock:
                result = ql.capture(Path('/fixture'), 'q', [], Path(d), 1, False)
                self.assertNotIn('KESTRELSEARCH_PROVIDER_TRACE_DIR', mock.call_args.kwargs['env'])
                self.assertEqual(result['exit_code'], 124)
                self.assertEqual(result['stderr'], 'timed out')

    def test_malformed_artifact_does_not_destroy_run(self):
        with tempfile.TemporaryDirectory() as d, patch('subprocess.run', return_value=subprocess.CompletedProcess([], 0, '[]', '')):
            (Path(d) / 'run-bad.json').write_text('{')
            run = ql.capture(Path('/fixture'), 'q', [], Path(d), 1, False)
            self.assertEqual(len(run['artifact_errors']), 1)
            self.assertEqual(run['exit_code'], 0)


class BatchTests(unittest.TestCase):
    def test_recover_runs_handles_missing_and_empty_files(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)/'runs.jsonl'
            self.assertEqual(ql.recover_runs(path), [])
            self.assertFalse(path.exists())
            path.touch()
            self.assertEqual(ql.recover_runs(path), [])

    def test_recover_runs_discards_only_unterminated_invalid_tail(self):
        for tail in [b'{"run_id":', b'{"query":"caf\xc3']:
            with self.subTest(tail=tail), tempfile.TemporaryDirectory() as d:
                path = Path(d)/'runs.jsonl'
                committed = b'{"run_id":"complete"}\n'
                path.write_bytes(committed + tail)
                with contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(ql.recover_runs(path), [dict(run_id='complete')])
                self.assertEqual(path.read_bytes(), committed)

    def test_recover_runs_preserves_complete_unterminated_record_before_append(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)/'runs.jsonl'
            path.write_bytes(b'{"run_id":"complete"}')
            self.assertEqual(ql.recover_runs(path), [dict(run_id='complete')])
            with path.open('a') as stream:
                stream.write('{"run_id":"next"}\n')
            self.assertEqual(ql.recover_runs(path), [dict(run_id='complete'), dict(run_id='next')])

    def test_recover_runs_rejects_committed_corruption_without_changing_file(self):
        for data in [b'{bad}\n', b'{bad}\n{"run_id":"complete"}\n', b'\xff\n', b'\n']:
            with self.subTest(data=data), tempfile.TemporaryDirectory() as d:
                path = Path(d)/'runs.jsonl'
                path.write_bytes(data)
                with self.assertRaises((ValueError, UnicodeDecodeError)):
                    ql.recover_runs(path)
                self.assertEqual(path.read_bytes(), data)

    def test_resume_skips_complete_runs_and_preserves_orphan_attempt(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            binary = root/'binary'
            binary.write_text('fixture')
            queries = root/'queries.json'
            queries.write_text(json.dumps([dict(id='fixture', query='fixture')]))
            output = root/'batch'
            argv = ['quality_latency.py', '--binary', str(binary), '--queries', str(queries),
                    '--output', str(output), '--stage', 'swisscows', '--rounds', '1']
            captured = dict(exit_code=0, results=[], artifacts=[], process_seconds=.1, command_seconds=.05)
            with patch.object(sys, 'argv', argv), patch('quality_latency.capture', return_value=captured.copy()) as capture, \
                 patch('subprocess.check_output', return_value='fixture'), patch('quality_latency.time.sleep'), \
                 contextlib.redirect_stdout(io.StringIO()):
                # git diff returns bytes; all version and commit calls request text.
                def output_for(*args, **kwargs):
                    return 'fixture' if kwargs.get('text') else b'fixture'
                with patch('subprocess.check_output', side_effect=output_for):
                    ql.main()
                    self.assertEqual(capture.call_count, 1)
                    metadata_before = (output/'metadata.json').read_bytes()
                    saved = json.loads((output/'runs.jsonl').read_text())
                    self.assertIn('attempt-1', saved['artifact_directory'])
                    with patch.object(sys, 'argv', argv+['--resume']):
                        ql.main()
                    self.assertEqual(capture.call_count, 1)
                    self.assertEqual((output/'metadata.json').read_bytes(), metadata_before)
                    # Simulate interruption while appending the captured record.
                    (output/'runs.jsonl').write_text('{"run_id":')
                    orphan = Path(saved['artifact_directory'])/'run-orphan.json'
                    orphan.write_text('{}')
                    with patch.object(sys, 'argv', argv+['--resume']), contextlib.redirect_stderr(io.StringIO()):
                        ql.main()
                    rerun = json.loads((output/'runs.jsonl').read_text())
                    self.assertIn('attempt-2', rerun['artifact_directory'])
                    self.assertTrue(orphan.exists())

    def test_invalid_options_fail_before_output_directory_creation(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stderr(io.StringIO()):
            output = Path(d)/'must-not-exist'
            with patch.object(sys, 'argv', ['quality_latency.py', '--binary', '/missing', '--output', str(output), '--stage', 'retrieval', '--cache', 'warm']):
                with self.assertRaises(SystemExit):
                    ql.main()
            self.assertFalse(output.exists())


class ReportTests(unittest.TestCase):
    def run_record(self, **changes):
        return dict(condition='test', query_id='q', process_seconds=1, command_seconds=.5,
                    exit_code=0, results=[dict(url='https://example.org', content=None)], artifacts=[], **changes)

    def test_insufficient_and_unknown_pool_are_distinct(self):
        for available, expected in [(5, False), (15, True), (20, True)]:
            run = {'artifacts': [dict(candidate_counts={'after_search': available}, candidates=[])]}
            self.assertEqual(qr.pool_check(run, 15)['sufficient'], expected)
        self.assertIsNone(qr.pool_check({}, 15)['sufficient'])

    def test_judgments_never_inferred_from_urls_or_counts(self):
        run = self.run_record()
        result = qr.summarize([run])['test']
        self.assertIsNone(result['precision_at_k'])
        self.assertIsNone(result['evidence_slot_coverage'])
        key = qr.judgment_key('q', run['results'][0])
        judged = qr.summarize([run], {key: dict(relevance=2, evidence=0)})['test']
        self.assertEqual(judged['precision_at_k'], .2)
        self.assertEqual(judged['evidence_slot_coverage'], 0)
        self.assertIsNone(judged['process_seconds']['p50_query_bootstrap_95_interval'])

    def test_content_changes_invalidate_evidence_judgment(self):
        self.assertNotEqual(qr.judgment_key('q', dict(url='u', content='a')),
                            qr.judgment_key('q', dict(url='u', content='b')))

    def test_empty_batch_summary_and_missing_command_time(self):
        self.assertEqual(qr.summarize([]), {})
        run = self.run_record()
        run['command_seconds'] = None
        self.assertEqual(qr.summarize([run])['test']['command_seconds']['missing'], 1)


class ExportTests(unittest.TestCase):
    def test_export_recomputes_metrics_and_detects_tampering(self):
        import export_quality_study
        import verify_quality_export
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            batch = root/'pilot'/'retrieval'
            batch.mkdir(parents=True)
            (batch/'metadata.json').write_text(json.dumps(dict(stage='retrieval', binary='/local/binary', query_file='/local/queries')))
            result = dict(url='https://example.org', content=None)
            run = dict(run_id='1-q-minimum-1', query_id='q', round=1, condition='minimum-1',
                       effective={'top_k': 5}, started_at='2026-09-13T00:00:00Z', process_seconds=1.25,
                       command_seconds=1.0, exit_code=0, pool_check={}, command=['/local/binary', 'search', 'q'],
                       results=[result], artifacts=[])
            (batch/'runs.jsonl').write_text(json.dumps(run)+'\n')
            key = qr.judgment_key('q', result)
            judgments = root/'judgments.json'
            judgments.write_text(json.dumps(dict(judgments=[dict(query_id='q', url=key[1], content_sha256=key[2],
                                 relevance=2, evidence=0, rationale='Fixture judgment.')])) )
            output = root/'export'
            with patch.object(sys, 'argv', ['export', '--pilot', str(root/'pilot'), '--judgments', str(judgments), '--output', str(output)]):
                export_quality_study.main()
            self.assertNotIn('/local/', (output/'metadata.json').read_text())
            with patch.object(sys, 'argv', ['verify', str(output)]), contextlib.redirect_stdout(io.StringIO()):
                verify_quality_export.main()
                with (output/'measurements.jsonl').open('a') as stream:
                    stream.write('\n')
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    verify_quality_export.main()


class UncertaintyTests(unittest.TestCase):
    def test_paired_identical_conditions_have_zero_difference(self):
        import report_quality_uncertainty as report
        runs = [dict(query_id=q, condition=c, effective={'top_k': 5},
                     results=[{'relevance': 2, 'evidence': 0}])
                for q in ['a', 'b'] for c in ['search-1s', 'search-default-5s']]
        self.assertEqual(report.intervals(runs)['search-1s']['precision_difference_from_default_5s'], [0, 0])

    def test_unknown_scores_do_not_get_quality_intervals(self):
        import report_quality_uncertainty as report
        runs = [dict(query_id=q, condition='test', effective={'top_k': 5},
                     results=[{'relevance': None, 'evidence': None}]) for q in ['a', 'b']]
        result = report.intervals(runs)['test']
        self.assertIsNone(result['precision_at_k'])
        self.assertIsNone(result['page_evidence_slot_coverage'])


if __name__ == '__main__':
    unittest.main()
