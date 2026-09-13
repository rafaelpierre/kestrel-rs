import unittest
from yahoo_retry_replay import replay


class RetryReplayTests(unittest.TestCase):
    def record(self, statuses):
        attempts, intervals = [], []
        for n, status in enumerate(statuses):
            aid = str(n)
            attempts.append(dict(attempt_id=aid, http_status=status, outcome='response' if status else 'transport_error'))
            intervals.append(dict(attempt_id=aid, phase='send', elapsed_ms=25, censored=False))
            if n < len(statuses)-1:
                intervals.append(dict(attempt_id=None, phase='backoff', elapsed_ms=100*(n+1), censored=False))
        return dict(attempts=attempts, intervals=intervals)

    def test_persistent_failure_counts_null_id_backoff_between_retained_sends(self):
        record = self.record([500, 500, 500])
        self.assertEqual([replay(record, c)['retained_attempt_ms'] for c in (1, 2, 3)], [25, 150, 375])
        self.assertEqual([replay(record, c)['sends_saved'] for c in (1, 2, 3)], [2, 1, 0])

    def test_caps_lose_late_recovery_and_transport_is_not_empty_success(self):
        for statuses in ([500, 200], [500, 500, 200], [None, 200]):
            record = self.record(statuses)
            self.assertFalse(replay(record, len(statuses)-1)['http_recovered'])
            self.assertTrue(replay(record, len(statuses))['http_recovered'])

    def test_censoring_and_no_attempts(self):
        self.assertEqual(replay(self.record([]), 1)['sends'], 0)
        record = self.record([None]);record['intervals'][0]['censored'] = True
        self.assertTrue(replay(record, 1)['censored'])
        record = self.record([500])
        record['intervals'].append(dict(attempt_id=None, phase='backoff', elapsed_ms=50, censored=True))
        self.assertEqual(replay(record, 1)['retained_attempt_ms'], 25)
        self.assertEqual(replay(record, 3)['retained_attempt_ms'], 75)
        self.assertTrue(replay(record, 3)['censored'])


if __name__ == '__main__':
    unittest.main()
