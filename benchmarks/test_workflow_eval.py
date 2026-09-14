"""Synthetic frozen evidence validates evaluator bookkeeping without model calls."""
import hashlib
import json
from pathlib import Path
import unittest

import workflow_eval as workflow


def fixture():
    text = 'Widget 2.0 preserves indentation when extracting code.'
    return {'schema_version': 1, 'task_id': 'fixture-01', 'arm': 'selective',
            'model': 'synthetic-no-model', 'plan_sha256': 'fixture',
            'binary_sha256': 'fixture', 'skill_sha256': 'fixture',
            'started_at': '2026-09-14T10:00:00Z', 'ended_at': '2026-09-14T10:00:03Z',
            'task_seconds': 3, 'status': 'completed', 'events': [
                {'sequence': 0, 'kind': 'model', 'start_seconds': 0, 'end_seconds': 1,
                 'request': {'task': 'What changed?'}, 'response': {'tool': 'fetch'}, 'usage': None},
                {'sequence': 1, 'kind': 'fetch', 'start_seconds': 1, 'end_seconds': 2,
                 'argv': ['kestrel', 'fetch', 'https://fixture.invalid/release', *workflow.FETCH],
                 'exit_code': 0, 'timed_out': False, 'stdout': text, 'stderr': '',
                 'decision': 'Read supplied release URL', 'agent_visible': text,
                 'page_attempts': 1, 'bytes_received': None,
                 'evidence': [{'id': 'e1', 'kind': 'page', 'url': 'https://fixture.invalid/release',
                               'text': text, 'text_sha256': hashlib.sha256(text.encode()).hexdigest()}]}],
            'final': {'answer': text, 'abstained': False,
                      'citations': [{'claim': text, 'evidence_id': 'e1', 'passage': text}]}}


def judgment(record):
    return {'record_sha256': workflow.digest(record), 'assessor': 'synthetic-test',
            'rationale': 'Bookkeeping test, not a factual evaluation',
            **{key: {'score': 2, 'rationale': 'Synthetic complete evidence'} for key in workflow.DIMENSIONS}}


class WorkflowTests(unittest.TestCase):
    def test_blinding_and_counterbalance(self):
        tasks = [{'id': 't1', 'question': 'Which change?', 'as_of': '2026-09-11',
                  'repository': 'owner/repo', 'split': 'development',
                  'answer': 'SECRET', 'snapshots': '/private/SECRET'}]
        plan = workflow.prepare(tasks, 'synthetic-no-model')
        self.assertNotIn('SECRET', str(plan))
        self.assertEqual(len(plan['schedule']), 6)
        for arm in workflow.ARMS:
            self.assertEqual([sum(row['order'][i] == arm for row in plan['schedule'])
                              for i in range(3)], [2, 2, 2])
        packet = workflow.packet(plan, 't1', 'selective')
        self.assertNotIn('split', packet)
        self.assertNotIn('repository', packet)
        self.assertEqual(plan['readiness']['isolation_adapter'], 'not-implemented')

    def test_actual_pilot_prepares_all_thirty_tasks(self):
        tasks = json.loads((Path(__file__).parent / 'coding-pilot-20260911/questions.json').read_text())
        plan = workflow.prepare(tasks, 'synthetic-no-model')
        self.assertEqual(len(plan['tasks']), 30)
        self.assertEqual(len(plan['schedule']), 180)
        self.assertEqual(sum(t['split'] == 'development' for t in plan['tasks']), 10)

    def test_repository_leakage_rejected(self):
        row = {'id': 'a', 'question': 'Q', 'as_of': 'date', 'repository': 'repo', 'split': 'development'}
        with self.assertRaisesRegex(ValueError, 'crosses splits'):
            workflow.prepare([row, {**row, 'id': 'b', 'split': 'held_out'}], 'synthetic')

    def test_unknown_usage_is_not_zero(self):
        result = workflow.validate(fixture())
        self.assertIsNone(result['metrics']['input_tokens'])
        self.assertIsNone(result['metrics']['bytes_received'])
        self.assertEqual(result['metrics']['task_seconds'], 3)
        self.assertEqual(result['metrics']['tool_seconds'], 1)
        self.assertEqual(result['metrics']['model_seconds'], 1)
        self.assertEqual(result['semantic_quality'], 'unjudged')

    def test_fabricated_or_hidden_citations_fail(self):
        for mutate in (
                lambda r: r['final']['citations'][0].update(passage='fabricated'),
                lambda r: r['events'][1].update(agent_visible='hidden'),
                lambda r: r['final']['citations'][0].update(evidence_id='unseen'),
                lambda r: r['events'][1]['evidence'][0].update(text_sha256='changed')):
            row = fixture()
            mutate(row)
            with self.assertRaises(ValueError):
                workflow.validate(row)

    def test_failed_fetch_is_retained_and_consumes_budget(self):
        row = fixture()
        row['events'][1].update(exit_code=124, timed_out=True, stdout='', agent_visible='', evidence=[])
        row['final'].update(answer='Unable to establish the change.', citations=[], abstained=True)
        row['status'] = 'timeout'
        result = workflow.score(row, judgment(row))
        self.assertEqual(result['metrics']['page_attempts'], 1)
        self.assertFalse(result['supported_complete_answer'])

    def test_unknown_quality_and_changed_trajectory_fail_closed(self):
        row = fixture()
        grades = judgment(row)
        grades['completeness']['score'] = None
        result = workflow.score(row, grades)
        self.assertFalse(result['fully_judged'])
        self.assertFalse(result['supported_complete_answer'])
        row['final']['answer'] = 'Different answer'
        with self.assertRaisesRegex(ValueError, 'does not match'):
            workflow.score(row, grades)

    def test_budget_excess_is_reported_not_dropped(self):
        row = fixture()
        row['task_seconds'] = 121
        result = workflow.score(row, judgment(row))
        self.assertEqual(result['budget_violations'], ['task_seconds'])
        self.assertFalse(result['supported_complete_answer'])

    def test_nonfinite_overlapping_and_missing_receipts_rejected(self):
        for mutate in (
                lambda r: r.update(task_seconds=float('nan')),
                lambda r: r['events'][1].update(start_seconds=.5),
                lambda r: r['events'][1].pop('exit_code'),
                lambda r: r['events'][1].update(page_attempts=0),
                lambda r: r.update(arm='closed-book')):
            row = fixture()
            mutate(row)
            with self.assertRaises(ValueError):
                workflow.validate(row)

    def test_workflow_contamination_rejected(self):
        row = fixture()
        row['events'][1].update(kind='search', mode='bundled',
                                argv=['kestrel', 'search', 'Widget release'], page_attempts=3)
        with self.assertRaisesRegex(ValueError, 'Wrong workflow'):
            workflow.validate(row)

    def test_partial_or_metadata_only_evidence_is_not_semantic_pass(self):
        row = fixture()
        row['events'][1]['evidence'][0]['kind'] = 'snippet'
        grades = judgment(row)
        grades['citation_support']['score'] = 1
        result = workflow.score(row, grades)
        self.assertFalse(result['supported_complete_answer'])
        self.assertEqual(result['quality']['correctness'], 2)

    def test_percentiles_use_nearest_rank(self):
        self.assertEqual(workflow.percentile([1, 2, 3, 20], .5), 2)
        self.assertEqual(workflow.percentile([1, 2, 3, 20], .95), 20)
        self.assertIsNone(workflow.percentile([], .5))


if __name__ == '__main__':
    unittest.main()
