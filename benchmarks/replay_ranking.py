#!/usr/bin/env python3
"""Replay candidate artifacts through the Rust ranker without additional retrieval."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', required=True, type=Path, help='Built rank_replay example')
    p.add_argument('--batch', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    args = p.parse_args()
    with args.output.open('x') as stream:
        for run in map(json.loads,(args.batch/'runs.jsonl').read_text().splitlines()):
            artifacts = run.get('artifacts')
            if artifacts is None:
                directory = Path(run.get('artifact_directory', args.batch/'artifacts'/run['run_id']))
                artifacts = [json.loads(p.read_text()) for p in directory.glob('run-*.json')]
            if len(artifacts) != 1:
                stream.write(json.dumps(dict(source_run_id=run['run_id'], query_id=run['query_id'],
                    round=run['round'], replay_status='missing_or_ambiguous_artifact'))+'\n')
                continue
            data = json.dumps(artifacts[0], sort_keys=True).encode()
            with tempfile.TemporaryDirectory() as directory:
                frozen = Path(directory)/'input.json'
                frozen.write_bytes(data)
                command = [str(args.binary.resolve()), str(frozen)]
                started = time.perf_counter()
                output = subprocess.check_output(command, text=True)
                elapsed = time.perf_counter() - started
            for ranked in map(json.loads,output.splitlines()):
                ranked.update(condition=ranked.pop('policy').lower(), query_id=run['query_id'], round=run['round'],
                              input_sha256=hashlib.sha256(data).hexdigest(),
                              source_run_id=run['run_id'], replay_binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                              all_policies_process_seconds=elapsed, command=command)
                stream.write(json.dumps(ranked)+'\n')


if __name__ == '__main__':
    main()
