import unittest
from empty_discovery import eligible


class RecoveryEligibility(unittest.TestCase):
    def test_only_deadline_units_retry(self):
        outcomes = [dict(query='q', engine=e, outcome=o) for e, o in
                    [('bing', 'deadline'), ('mojeek', 'challenge'),
                     ('yahoo', 'request_error'), ('yep', 'empty')]]
        self.assertEqual(eligible([], outcomes, ['q']), [('q', ['bing'])])

    def test_exhausted_empty_unknown_and_cancelled_do_not_retry(self):
        for outcome in ('empty', 'filtered_empty', 'challenge', 'request_error',
                        'cancelled_caller', 'cancelled_min_results', 'unknown'):
            with self.subTest(outcome=outcome):
                self.assertEqual(eligible([], [dict(query='q', engine='bing', outcome=outcome)], ['q']), [])
        self.assertEqual(eligible([], [], ['q']), [])

    def test_multi_query_provenance_keeps_successful_queries(self):
        outcomes = [dict(query=q, engine='bing', outcome='deadline') for q in ['a', 'b', 'c']]
        results = [dict(query='a', sources=[dict(query='a'), dict(query='b')])]
        self.assertEqual(eligible(results, outcomes, ['a', 'b', 'c']), [('c', ['bing'])])

    def test_rate_limit_during_deadline_does_not_restart(self):
        for attempt in ({'http_status': 429, 'retry_after': '30'},
                        {'challenge': 'detected'}, {'http_status': 403}):
            self.assertEqual(eligible([], [dict(query='q', engine='bing', outcome='deadline',
                             lifecycle={'attempts': [attempt]})], ['q']), [])

    def test_partial_success_does_not_refill(self):
        self.assertEqual(eligible([dict(query='q')],
                         [dict(query='q', engine='bing', outcome='deadline')], ['q']), [])


if __name__ == '__main__':
    unittest.main()
