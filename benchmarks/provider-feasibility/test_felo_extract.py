import copy
import json
from pathlib import Path
import unittest
from felo_extract import extract

FIXTURE = Path(__file__).resolve().parents[2] / 'tests/fixtures/providers/felo-thread-sanitized.html'


def wrap(thread):
    return '<script id="__NEXT_DATA__" type="application/json">' + json.dumps({'props': {'pageProps': {'threads': [thread]}}}) + '</script>'


class ExtractTests(unittest.TestCase):
    def test_observed_source_order_and_content(self):
        result = extract(FIXTURE.read_text())[0]
        self.assertEqual(result['query'], 'Rust E0382 use of moved value')
        self.assertEqual(len(result['results']), 11)
        self.assertEqual(result['results'][0]['engine_rank'], 1)
        self.assertEqual(result['results'][6]['url'], 'https://doc.rust-lang.org/error_codes/E0382.html')
        self.assertIn('Cookie Settings', result['results'][0]['snippet'])

    def test_reject_entry_empty_incomplete_and_malformed(self):
        for html in ['<html>Navigation</html>', '<script id="__NEXT_DATA__">{broken</script>', wrap({'status': 'running'}), wrap({'status': 'completed', 'query': 'x', 'recall_contexts': []})]:
            with self.subTest(html=html), self.assertRaises(ValueError):
                extract(html)

    def test_missing_fields_and_invalid_urls(self):
        thread = {'status': 'completed', 'query': 'C++ café 日本語 "x" site:example.org', 'recall_contexts': [{'link': 'https://example.org'}]}
        row = extract(wrap(thread))[0]['results'][0]
        self.assertIsNone(row['title'])
        self.assertIsNone(row['snippet'])
        self.assertEqual(row['query'], thread['query'])
        for value in ['javascript:alert(1)', 'https://', 123]:
            invalid = copy.deepcopy(thread)
            invalid['recall_contexts'][0]['link'] = value
            with self.assertRaises(ValueError):
                extract(wrap(invalid))


if __name__ == '__main__':
    unittest.main()
