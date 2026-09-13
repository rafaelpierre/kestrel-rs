import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from streaming_validation_report import distribution, summarize


class ReportTests(unittest.TestCase):
    def metadata(self):
        return {"corpus": {"queries": [{"id": "q"}]}, "query_limit": 1,
                "trials": 1, "enabled_providers": ["bing", "yahoo"]}

    def sample_run(self, policy="stream", urls=("https://a",), error=None):
        return {"query_id": "q", "trial": 1, "fresh_clients": True, "policy": policy,
                "elapsed_ms": 12, "providers": {}, "diagnostics": [], "error": error,
                "collector": {"first_five_unique_ms": None, "threshold_observed_ms": None}, "contributing_providers": [],
                "threshold_met_at_return": False, "final_keys": list(urls), "final_results": [{"url": url} for url in urls]}

    def test_missing_measurements_and_empty_overlap_are_not_success(self):
        # No observations must remain null; unseen providers remain in the matrix.
        self.assertEqual(distribution([]), {"n": 0, "p50": None, "p95": None})
        summary = summarize(self.metadata(), [self.sample_run("full", ()), self.sample_run(urls=())])
        self.assertFalse(summary["complete"])
        self.assertEqual(len(summary["provider_matrix"]), 2)
        group = summary["groups"][2]
        self.assertEqual(group["top_five_slot_overlap_vs_full"]["p50"], 0)
        self.assertIsNone(group["time_to_five_ms"]["p50"])

    def test_unknown_judgments_and_errors_do_not_become_zero_relevance(self):
        summary = summarize(self.metadata(), [self.sample_run()])
        group = summary["groups"][2]
        self.assertEqual(group["unjudged_returned_results"], 1)
        self.assertIsNone(group["precision_at_five_fully_judged_runs"]["p50"])
        judged = summarize(self.metadata(), [self.sample_run()], {"q": {"https://a": True}})
        self.assertEqual(judged["groups"][2]["precision_at_five_fully_judged_runs"]["p50"], .2)
        failed = summarize(self.metadata(), [self.sample_run(urls=(), error="deadline")])
        self.assertIsNone(failed["groups"][2]["precision_at_five_fully_judged_runs"]["p50"])

    def test_help_envelope_is_accepted_by_the_report_cli(self):
        script = Path(__file__).with_name("streaming_validation_report.py")
        help_text = subprocess.check_output([sys.executable, str(script), "--help"], text=True)
        compact = " ".join(help_text.split())
        envelope = json.loads(re.search(r"JSON envelope: (.*?);", compact).group(1))
        self.assertEqual(envelope, {"version": 1, "judgments": {"query_id": {"url": True}}})
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "metadata.json").write_text(json.dumps(self.metadata()))
            (directory / "runs.jsonl").write_text(json.dumps(self.sample_run()) + "\n")
            (directory / "judgments.json").write_text(json.dumps(envelope))
            subprocess.run([sys.executable, str(script), "--input", str(directory),
                            "--output", str(directory / "summary.json"),
                            "--judgments", str(directory / "judgments.json")],
                           check=True, capture_output=True, text=True)
            summary = json.loads((directory / "summary.json").read_text())
            self.assertEqual(summary["groups"][2]["unjudged_returned_results"], 1)
            self.assertIsNotNone(summary["judgments_sha256"])

    def test_duplicate_or_unscheduled_runs_fail(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            summarize(self.metadata(), [self.sample_run(), self.sample_run()])
        with self.assertRaisesRegex(ValueError, "unexpected"):
            summarize(self.metadata(), [self.sample_run("unknown")])


if __name__ == "__main__":
    unittest.main()
