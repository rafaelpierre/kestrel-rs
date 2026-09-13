#!/usr/bin/env python3
"""Run Rust and Python tests under per-test OTEL contexts; no Python SDK required.

One process per Rust test makes attribution exact without mutating environment
variables inside concurrent tests. Use ordinary cargo test for contention checks.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--include-ignored', action='store_true')
    parser.add_argument('--filter', default='')
    parser.add_argument('--inner', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--casefile', help=argparse.SUPPRESS)
    args = parser.parse_args()
    os.chdir(ROOT)
    helper = ROOT / 'target/debug/examples/trace_command'
    if args.casefile:
        failed = []
        for case in json.loads(Path(args.casefile).read_text()):
            if subprocess.call([str(helper), case['name'], *case['command']], cwd=case['cwd']):
                failed.append(case['name'])
        return bool(failed)
    if not args.inner:
        subprocess.run(['cargo', 'build', '--locked', '--example', 'trace_command'], check=True)
        env = dict(os.environ)
        env.setdefault('KESTREL_BENCH_TEST_BINARY', str(ROOT/'target/debug/kestrel'))
        env.setdefault('KESTRELSEARCH_OTEL_RUN_ID', str(uuid.uuid4()))
        env.setdefault('OTEL_TRACES_SAMPLER', 'always_on')
        if env.get('OTEL_TRACES_SAMPLER') != 'always_on':
            parser.error('test runs require OTEL_TRACES_SAMPLER=always_on')
        env.setdefault('KESTRELSEARCH_OTEL_CONTENT', 'sanitized')
        if not (env.get('OTEL_EXPORTER_OTLP_ENDPOINT') or env.get('OTEL_EXPORTER_OTLP_TRACES_ENDPOINT')):
            print('Honeycomb export unavailable: no OTLP endpoint configured', file=sys.stderr)
        return subprocess.call([str(helper), 'test.run', sys.executable, __file__, *sys.argv[1:], '--inner'], env=env)
    build = subprocess.run(['cargo', 'test', '--locked', '--all-features', '--no-run', '--message-format=json'], text=True, stdout=subprocess.PIPE, check=True)
    binaries = sorted({row['executable'] for line in build.stdout.splitlines() if (row := json.loads(line)).get('reason') == 'compiler-artifact' and row.get('profile', {}).get('test') and row.get('executable')})
    failed = []
    executed = skipped = 0
    def suite(name, cases):
        with tempfile.TemporaryDirectory() as tmp:
            casefile = Path(tmp) / 'cases.json'
            casefile.write_text(json.dumps(cases))
            return subprocess.call([str(helper), 'suite:'+name, sys.executable, __file__, '--inner', '--casefile', str(casefile)])

    for binary in binaries:
        cases = []
        listing = subprocess.check_output([binary, '--list', '--format=terse'], text=True)
        ignored_listing = subprocess.check_output([binary, '--list', '--ignored', '--format=terse'], text=True)
        ignored = {line[:-6] for line in ignored_listing.splitlines() if line.endswith(': test')}
        for line in listing.splitlines():
            if not line.endswith(': test'):
                continue
            test = line[:-6]
            if args.filter not in test:
                continue
            name = f'{Path(binary).name}/{test}'
            if test in ignored and not args.include_ignored:
                command = ['-']
                skipped += 1
            else:
                command = [binary, '--exact', test, '--nocapture'] + (['--ignored'] if test in ignored else [])
                executed += 1
            cases.append({'name': name, 'command': command, 'cwd': str(ROOT)})
        if cases and suite(Path(binary).name, cases):
            failed.append(Path(binary).name)
    # unittest owns each Python test case; discover IDs, then isolate each case
    # in its source directory so existing benchmark sibling imports still work.
    for directory in (ROOT / 'benchmarks', ROOT / 'benchmarks/budget-overhead'):
        cases = []
        # Keep discovery independent of private unittest internals.
        code = 'import unittest\ndef walk(s):\n for x in s:\n  if isinstance(x,unittest.TestSuite): yield from walk(x)\n  else: yield x.id()\nprint("\\n".join(walk(unittest.defaultTestLoader.discover(".",pattern="test_*.py"))))'
        for test in subprocess.check_output([sys.executable, '-c', code], cwd=directory, text=True).splitlines():
            if args.filter not in test:
                continue
            executed += 1
            cases.append({'name': test, 'command': [sys.executable, str(ROOT/'scripts/trace_unittest.py'), test], 'cwd': str(directory)})
        if cases and suite(str(directory.relative_to(ROOT)), cases):
            failed.append(str(directory))
    if not args.filter:
        if subprocess.call([str(helper), 'rust.doc_tests', 'cargo', 'test', '--locked', '--all-features', '--doc']):
            failed.append('rust.doc_tests')
    summary = {'executed': executed, 'skipped': skipped, 'failed': failed, 'expected_case_spans': executed + skipped + int(not args.filter)}
    if manifest := os.environ.get('KESTRELSEARCH_OTEL_MANIFEST'):
        Path(manifest).write_text(json.dumps(summary))
    print(json.dumps(summary, indent=2))
    return bool(failed)

if __name__ == '__main__':
    sys.exit(main())
