"""Benchmark consumers preserve result records across the CLI JSON migration."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import cli_compare
import quality_latency
import query_semantics


class JsonOutputTests(unittest.TestCase):
    def test_legacy_and_envelope_populated_and_empty_results(self):
        for results in [[], [{"url": "https://example.com/", "title": "Example"}]]:
            for payload in [results, {"results": results, "elapsed_seconds": 0.125}]:
                with self.subTest(payload=payload), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    binary = root / "kestrel"
                    binary.write_text("fixture")
                    response = subprocess.CompletedProcess([], 0, json.dumps(payload), "")

                    def fake_run(*args, **kwargs):
                        env = kwargs.get("env", {})
                        if "KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR" in env:
                            artifact = Path(env["KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR"])
                            artifact.mkdir(parents=True, exist_ok=True)
                            (artifact / (env["KESTRELSEARCH_BENCHMARK_RUN_ID"] + "-test.json")).write_text("{}")
                        return response

                    with patch("subprocess.run", side_effect=fake_run):
                        comparison = cli_compare.run_search("rust", binary,
                            {"id": "test", "query": "example"}, 1, [], "fanout", 1, root / "compare")
                        self.assertEqual(comparison["result_count"], len(results))
                        self.assertEqual(comparison["urls"], [r["url"] for r in results])
                        capture = quality_latency.capture(binary, "example", [], root / "quality", 1, False)
                        self.assertEqual(capture["exit_code"], 0)
                        self.assertEqual(capture["results"], results)
                        output = root / "semantics.json"
                        with patch.object(sys, "argv", ["query_semantics.py", "--binary", str(binary),
                                "--output", str(output), "--engines", "bing", "--stages", "no-fetch",
                                "--query", "example"]):
                            query_semantics.main()
                        run = json.loads(output.read_text())["runs"][0]
                        self.assertEqual(run["results"], results)
                        self.assertEqual(run["outcome"], "results" if results else "empty")


if __name__ == "__main__":
    unittest.main()
