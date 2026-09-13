import contextlib
import io
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


class QueryReplayTests(unittest.TestCase):
    def test_timeout_keeps_partial_output_and_source_identity(self):
        runner = Path(__file__).parent / 'query-defaults' / 'run.py'
        original_run = subprocess.run
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'kestrel'
            binary.write_text('#!/bin/sh\necho kestrel-test\n')
            binary.chmod(0o755)
            output = root / 'results'

            def capture(args, **kwargs):
                if Path(args[0]) == binary.resolve() and args[1] == 'search':
                    raise subprocess.TimeoutExpired(args, 30, b'partial stdout', b'partial stderr')
                return original_run(args, **kwargs)

            with patch.object(sys, 'argv', [str(runner), '--binary', str(binary),
                                           '--output', str(output), '--stage', 'initial']), \
                    patch('subprocess.run', side_effect=capture), contextlib.redirect_stdout(io.StringIO()):
                runpy.run_path(str(runner), run_name='__main__')
            metadata = json.loads((output / 'metadata.json').read_text())
            self.assertEqual(len(metadata['revision']), 40)
            for key in ['tracked_diff_sha256', 'dataset_sha256', 'sha256']:
                self.assertEqual(len(metadata[key]), 64)
            records = [json.loads(line) for line in (output / 'calls.jsonl').read_text().splitlines()]
            self.assertEqual(len(records), 10)
            for record in records:
                self.assertEqual(record['stdout'], 'partial stdout')
                self.assertEqual(record['stderr'], 'partial stderr')
                self.assertTrue(record['outer_timeout'])
                self.assertEqual(record['exit_code'], 124)
                self.assertNotIn('--query-syntax', record['args'])
