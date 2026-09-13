import unittest
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from bing_fidelity import judgment_key, judgment_template, score, load_runs

class ScoringTests(unittest.TestCase):
    def row(self, results, **kw):
        return dict(variant='baseline', query='intent', results=results, elapsed_ms=20,
                    budget_ms=3000, **kw)

    def test_scheduled_failures_remain_in_coverage_denominator(self):
        result = dict(url='https://example.org',title='answer',snippet='evidence')
        rows = [self.row([result]), self.row([], error='deadline')]
        labels = {judgment_key('intent',result):dict(relevant=True,reason='addresses intent')}
        s = score(rows,labels)['baseline']
        self.assertEqual(s['usable_coverage'], .5)
        self.assertEqual(s['conditional_precision_at_5'], .2)
        self.assertEqual(s['errors'], 1)

    def test_missing_judgments_are_not_false_or_optimistic_precision(self):
        r = dict(url='https://example.org',title='answer',snippet='evidence')
        s = score([self.row([r])], {})['baseline']
        self.assertIsNone(s['conditional_precision_at_5'])
        self.assertIsNone(s['usable_coverage'])
        self.assertEqual(s['usable_coverage_bounds'], [0,1])

    def test_judgments_are_query_and_observation_specific(self):
        r = dict(url='https://example.org',title='answer',snippet='one')
        self.assertNotEqual(judgment_key('a',r),judgment_key('b',r))
        self.assertNotEqual(judgment_key('a',r),judgment_key('a',dict(r,snippet='two')))
        self.assertIsNone(next(iter(judgment_template([self.row([r])])['judgments'].values()))['relevant'])

    def test_invalid_judgments_fail_closed(self):
        r = dict(url='https://example.org',title='answer',snippet='one')
        for label in [dict(relevant=1,reason='bad type'),dict(relevant=True,reason='')]:
            with self.assertRaises(ValueError):
                score([self.row([r])],{judgment_key('intent',r):label})

    def test_replays_include_failed_captures_without_double_counting_network(self):
        result = dict(url='https://example.org', title='answer', snippet='evidence')
        rows = [dict(self.row([result]), id='ok', variant='bing-standard', attempts=1,
                     paired_views={'native': {'results': [result]}, 'portable': {'results': []}}),
                dict(self.row([], error='deadline'), id='failed', variant='bing-standard', attempts=1,
                     paired_views={'native': {'results': []}, 'portable': {'results': []}})]
        with TemporaryDirectory() as directory:
            path = Path(directory) / 'runs.json'
            path.write_text(json.dumps(dict(schema=1, experiment_revision=2,
                                            completed=True, expected_rows=2, runs=rows)))
            loaded = load_runs([path])
            self.assertEqual(len(loaded), 6)
            labels = {judgment_key('intent', result): dict(relevant=True, reason='addresses intent')}
            summaries = score(loaded, labels)
            native = summaries['bing-standard-native-replay']
            self.assertEqual(native['scheduled'], 2)
            self.assertEqual(native['usable_coverage'], .5)
            self.assertEqual(native['errors'], 1)
            self.assertEqual(native['isolated_attempts'], 0)
            self.assertIsNone(native['p50_ms'])
            self.assertIsNone(native['p95_ms'])
            self.assertEqual(summaries['bing-standard']['isolated_attempts'], 2)
            del rows[1]['paired_views']
            path.write_text(json.dumps(dict(schema=1, experiment_revision=2,
                                            completed=True, expected_rows=2, runs=rows)))
            with self.assertRaisesRegex(ValueError, 'missing paired views'):
                load_runs([path])

    def test_revision_three_passthrough_replay_and_missing_views(self):
        row = dict(self.row([], error='deadline'), id='failed', variant='bing-standard',
                   paired_views={'passthrough': {'results': []}})
        with TemporaryDirectory() as directory:
            path = Path(directory) / 'runs.json'
            payload = dict(schema=1, experiment_revision=3, completed=True,
                           expected_rows=1, runs=[row])
            path.write_text(json.dumps(payload))
            loaded = load_runs([path])
            self.assertEqual(len(loaded), 2)
            self.assertIn('bing-standard-passthrough-replay', score(loaded, {}))
            row['paired_views'] = {'native': {'results': []}, 'portable': {'results': []}}
            path.write_text(json.dumps(payload))
            with self.assertRaises(ValueError):
                load_runs([path])

    def test_completed_window_cannot_hide_incomplete_schedule(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            window = root / 'window-1'
            window.mkdir()
            path = window / 'runs.json'
            path.write_text(json.dumps(dict(schema=1, completed=True, expected_rows=0, runs=[])))
            (root / 'schedule.json').write_text(json.dumps(dict(
                completed=False, windows=2, executions=[dict(exit_code=0)])))
            with self.assertRaisesRegex(ValueError, 'incomplete schedule'):
                load_runs([path])

    def complete_schedule(self, root):
        root.mkdir(parents=True, exist_ok=True)
        (root / 'schedule.json').write_text(json.dumps(dict(
            completed=True, windows=2,
            executions=[dict(window=i, exit_code=0) for i in [1, 2]])))
        paths = []
        for i in [1, 2]:
            path = root / f'window-{i}' / 'runs.json'
            path.parent.mkdir()
            path.write_text(json.dumps(dict(schema=1, completed=True, expected_rows=1,
                                            runs=[self.row([])])))
            paths.append(path)
        return paths

    def test_requires_exactly_one_artifact_for_every_scheduled_window(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            first, second = self.complete_schedule(root)
            alias = root / 'alias.json'
            alias.symlink_to(first)
            extra = root / 'window-3' / 'runs.json'
            extra.parent.mkdir()
            extra.write_text(first.read_text())
            for paths in [[first], [first, first], [first, alias],
                          [first, extra], [first, second, extra]]:
                with self.subTest(paths=paths), self.assertRaises(ValueError):
                    load_runs(paths)
            loaded = load_runs([second, first])
            self.assertEqual(score(loaded, {})['baseline']['scheduled'], 2)

    def test_multiple_schedules_must_each_be_complete(self):
        with TemporaryDirectory() as directory:
            first = self.complete_schedule(Path(directory) / 'a')
            second = self.complete_schedule(Path(directory) / 'b')
            self.assertEqual(len(load_runs(first + second)), 4)
            with self.assertRaises(ValueError):
                load_runs(first + second[:1])

    def test_duplicate_standalone_alias_is_rejected(self):
        with TemporaryDirectory() as directory:
            path = Path(directory) / 'runs.json'
            path.write_text(json.dumps(dict(schema=1, completed=True, expected_rows=1,
                                            runs=[self.row([])])))
            alias = path.with_name('alias.json')
            alias.symlink_to(path)
            with self.assertRaises(ValueError):
                load_runs([path, alias])


class SanitizerTests(unittest.TestCase):
    def test_retains_organic_structure_and_strips_tracking_and_active_content(self):
        from sanitize_bing_fixture import sanitize
        raw = '<title>full query</title><li class="b_algo" data-id="secret"><h2><a href="https://user:password@example.org/result?token=secret#secret" onclick="secret()">Title</a></h2><div class="b_caption"><p>Snippet</p></div><script>secret()</script></li>'
        clean = sanitize(raw)
        self.assertNotIn('secret',clean)
        self.assertNotIn('password',clean)
        self.assertNotIn('<script',clean)
        self.assertIn('class="b_algo"',clean)
        self.assertIn('https://example.org/result',clean)
        self.assertIn('<p>Snippet</p>',clean)

if __name__ == '__main__':
    unittest.main()
