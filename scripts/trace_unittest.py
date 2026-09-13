#!/usr/bin/env python3
"""Execute one discovered unittest and report skips to the Rust trace wrapper."""
import json
import os
from pathlib import Path
import sys
import unittest

sys.path.insert(0, os.getcwd())
suite = unittest.defaultTestLoader.loadTestsFromName(sys.argv[1])
result = unittest.TextTestRunner(verbosity=2).run(suite)
if receipt := os.environ.get('KESTRELSEARCH_OTEL_TEST_OUTCOME'):
    Path(receipt).write_text(json.dumps({'skipped': len(result.skipped), 'executed': result.testsRun}))
sys.exit(not result.wasSuccessful())
