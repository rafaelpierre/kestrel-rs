"""Deterministic checks for metric denominators and unjudged evidence."""
import unittest
from score_retrieval import summarize


class ScoringTests(unittest.TestCase):
    def test_missing_slots_count_as_zero(self):
        run = {'query_id':'q', 'results':[{'url':'https://docs.example.org/x'}]}
        result = summarize([run], {('q','https://docs.example.org/x'):2}, {'q':'example.org'})
        self.assertEqual(result['precision_at_5'], .2)
        self.assertEqual(result['returned_domain_compliance'], 1)
        self.assertEqual(result['domain_slot_coverage'], .2)

    def test_unknown_is_not_automatically_scored_irrelevant(self):
        result = summarize([{'query_id':'q','results':[{'url':'https://new.example/'}]}], {}, {})
        self.assertIsNone(result['precision_at_5'])
        self.assertEqual(len(result['unjudged_pairs']), 1)

    def test_domain_suffix_attack_does_not_match(self):
        result = summarize([{'query_id':'q','results':[{'url':'https://example.org.evil.test/'}]}],
                           {('q','https://example.org.evil.test/'):0}, {'q':'example.org'})
        self.assertEqual(result['returned_domain_compliance'], 0)


if __name__ == '__main__':
    unittest.main()
