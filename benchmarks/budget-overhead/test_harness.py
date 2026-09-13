"""Check fixture semantics independently of provider availability."""
import http.client
import json
import pathlib
import tempfile
import threading
import unittest

from run import Fixture
from summarize import distribution, summarize
from http.server import ThreadingHTTPServer


class HarnessTests(unittest.TestCase):
    def test_fixtures_include_empty_result_page_and_unfinished_body(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            for path, expected in [("/bing?q=empty", b"b_no"),
                                   ("/bing?q=page", b"b_algo"),
                                   ("/page", b"fixture evidence")]:
                connection = http.client.HTTPConnection(*server.server_address, timeout=2)
                connection.request("GET", path)
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertIn(expected, response.read())
                connection.close()
            connection = http.client.HTTPConnection(*server.server_address, timeout=0.1)
            connection.request("GET", "/bing?q=hang")
            response = connection.getresponse()
            self.assertEqual(response.read(12), b"<html><body>")
            with self.assertRaises(TimeoutError):
                response.read()
            connection.close()
        finally:
            server.shutdown()
            server.server_close()

    def test_summary_rejects_unexpected_cli_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = pathlib.Path(directory)
            (folder / "metadata.json").write_text(json.dumps({"args": {"live": False, "baseline": False}}))
            (folder / "runs.json").write_text(json.dumps([{
                "kind": "cli", "scenario": "page", "fetch": True, "trace": False,
                "events": [], "returncode": 2,
            }]))
            with self.assertRaises(AssertionError):
                summarize(folder)

    def test_nearest_rank_percentile_and_units(self):
        result = distribution([i / 1000 for i in range(1, 21)])
        self.assertEqual(result, {"n": 20, "min": 1, "p50": 10.5, "p95": 19, "max": 20})


if __name__ == "__main__":
    unittest.main()
