import unittest
from bing_fidelity import judgment_key, judgment_template, score

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
