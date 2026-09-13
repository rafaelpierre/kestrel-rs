#!/usr/bin/env python3
"""Verify exported hashes and recompute core report metrics without captured pages."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import statistics

from quality_report import quantile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('report', type=Path)
    args = p.parse_args()
    for filename, expected in json.loads((args.report/'manifest.json').read_text()).items():
        actual = hashlib.sha256((args.report/filename).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f'checksum mismatch: {filename}')
    expected = json.loads((args.report/'summary.json').read_text())
    groups = defaultdict(list)
    for r in map(json.loads, (args.report/'measurements.jsonl').read_text().splitlines()):
        groups[(r['stage'], r['condition'])].append(r)
    for (stage, condition), runs in groups.items():
        summary = expected[stage][condition]
        checks = dict(runs=len(runs), errors=sum(r['exit_code'] != 0 for r in runs),
                      empty_rate=sum(not r['results'] for r in runs)/len(runs),
                      returned_slots=sum(len(r['results']) for r in runs))
        slots = sum(r['effective']['top_k'] for r in runs)
        for metric, field in [('precision_at_k', 'relevance'), ('evidence_slot_coverage', 'evidence')]:
            values = [result[field] for r in runs for result in r['results']]
            checks[metric] = None if None in values else sum(v == (2 if field == 'relevance' else 1) for v in values)/slots
        for key, value in checks.items():
            if summary[key] != value:
                raise ValueError(f'{stage}/{condition}: {key} mismatch')
        for field in ['process_seconds', 'command_seconds']:
            values = [r[field] for r in runs if r[field] is not None]
            for key, value in [('n', len(values)), ('p50', statistics.median(values) if values else None), ('p95', quantile(values, .95))]:
                if summary[field][key] != value:
                    raise ValueError(f'{stage}/{condition}: {field}/{key} mismatch')
    print(f'Verified {len(groups)} condition summaries and all exported file hashes.')


if __name__ == '__main__':
    main()
