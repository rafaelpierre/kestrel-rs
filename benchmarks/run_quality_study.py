#!/usr/bin/env python3
"""Reproduce the version-1 seven-query pilot for #75, one subprocess at a time."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
QUERY_IDS = ['music-bss', 'music-metric', 'music-sonic', 'music-duet', 'q01', 'q03', 'q08']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args()
    if args.output.exists() and not args.resume:
        parser.error('output already exists; use a fresh pilot directory')
    args.output.mkdir(parents=True, exist_ok=True)
    matrix = [('retrieval', 3, 5), ('search-budget', 5, 5), ('fetching', 2, 15), ('candidates', 3, 15)]
    for stage, rounds, minimum in matrix:
        output = args.output/stage
        command = [sys.executable, str(ROOT/'quality_latency.py'), '--binary', str(args.binary.resolve()),
                   '--output', str(output.resolve()), '--stage', stage, '--rounds', str(rounds),
                   '--min-results', str(minimum), '--candidate-pool', '15', '--top-k', '5',
                   '--query-syntax', 'portable', '--pace', '0.25', '--timeout', '50', '--cache', 'off', '--trace']
        for query in QUERY_IDS:
            command += ['--query-id', query]
        if args.resume and output.exists():
            command += ['--resume']
        with (args.output/'commands.jsonl').open('a') as stream:
            stream.write(json.dumps(command)+'\n')
        subprocess.run(command, check=True)


if __name__ == '__main__':
    main()
