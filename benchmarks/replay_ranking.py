#!/usr/bin/env python3
"""Replay candidate artifacts through the Rust ranker without additional retrieval."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', required=True, type=Path, help='Built rank_replay example')
    p.add_argument('--batch', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    args = p.parse_args()
    with args.output.open('x') as stream:
        for run in map(json.loads,(args.batch/'runs.jsonl').read_text().splitlines()):
            candidates = list((args.batch/'artifacts'/run['run_id']).glob('run-*.json'))
            if len(candidates) != 1:
                raise ValueError(f'{run["run_id"]}: expected exactly one candidate artifact')
            output = subprocess.check_output([str(args.binary.resolve()),str(candidates[0])],text=True)
            for ranked in map(json.loads,output.splitlines()):
                ranked.update(condition=ranked.pop('policy').lower(), query_id=run['query_id'], round=run['round'],
                              input_sha256=hashlib.sha256(candidates[0].read_bytes()).hexdigest())
                stream.write(json.dumps(ranked)+'\n')


if __name__ == '__main__':
    main()
