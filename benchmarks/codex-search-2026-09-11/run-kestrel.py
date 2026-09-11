import json
import subprocess
import sys
import time

start = time.perf_counter()
try:
    result = subprocess.run(
        ["kestrel", "search", sys.argv[1], "--mode", "fanout", "--provider-quorum", "1",
         "--no-fetch", "--no-rank", "-k", "5", "--output", "json"],
        capture_output=True, text=True, timeout=45,
    )
    print(json.dumps({"process_seconds": time.perf_counter() - start,
                      "exit_code": result.returncode, "stdout": result.stdout,
                      "stderr": result.stderr}))
except subprocess.TimeoutExpired:
    print(json.dumps({"process_seconds": time.perf_counter() - start,
                      "exit_code": 124, "stdout": "", "stderr": "45s timeout"}))
