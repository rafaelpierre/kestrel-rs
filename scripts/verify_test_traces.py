#!/usr/bin/env python3
"""Exercise the traced test runner against a loopback OTLP/JSON receiver.

This checks local delivery and ancestry, not Honeycomb ingestion. Raw payloads
stay in memory; the optional output is a payload-free summary.
"""
import argparse
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--summary')
    args = parser.parse_args()
    spans = []
    lock = threading.Lock()
    class Receiver(BaseHTTPRequestHandler):
        def do_POST(self):
            data = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            batch = [span for resource in data.get('resourceSpans', []) for scope in resource.get('scopeSpans', []) for span in scope.get('spans', [])]
            with lock:
                spans.extend(batch)
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(b'{}')
        def log_message(self, *_):
            pass
    server = ThreadingHTTPServer(('127.0.0.1',0), Receiver)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    env = {k:v for k,v in os.environ.items() if not k.startswith(('OTEL_', 'KESTRELSEARCH_OTEL_')) and k not in ('TRACEPARENT','TRACESTATE')}
    env.update(OTEL_EXPORTER_OTLP_ENDPOINT=f'http://127.0.0.1:{server.server_port}', OTEL_EXPORTER_OTLP_PROTOCOL='http/json', OTEL_TRACES_SAMPLER='always_on', KESTRELSEARCH_OTEL_CONTENT='sanitized')
    root = Path(__file__).resolve().parents[1]
    env['KESTREL_BENCH_TEST_BINARY'] = str(root / 'target/debug/kestrel')
    manifest_dir = tempfile.TemporaryDirectory()
    manifest_file = Path(manifest_dir.name)/"manifest.json"
    env["KESTRELSEARCH_OTEL_MANIFEST"] = str(manifest_file)
    try:
        code = subprocess.call([sys.executable, str(root/'scripts/test_traces.py'), '--filter', args.filter], env=env, cwd=root)
    finally:
        server.shutdown()
        server.server_close()
    ids = {s['spanId'] for s in spans}
    orphans = [s['name'] for s in spans if s.get('parentSpanId') and s['parentSpanId'] not in ids]
    counts = Counter(s['name'] for s in spans)
    manifest = json.loads(manifest_file.read_text()) if manifest_file.exists() else {}
    manifest_dir.cleanup()
    summary = {'command_exit': code, 'span_count': len(spans), 'trace_count': len({s['traceId'] for s in spans}), 'span_names': dict(counts), 'orphan_count': len(orphans), 'case_manifest': manifest}
    print(json.dumps(summary, indent=2))
    if args.summary:
        Path(args.summary).write_text(json.dumps(summary,indent=2)+'\n')
    if not spans or orphans or counts['kestrel.test'] != manifest.get('expected_case_spans'):
        return 1
    return code

if __name__ == '__main__':
    sys.exit(main())
