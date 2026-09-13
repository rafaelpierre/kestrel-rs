#!/usr/bin/env python3
"""Summarize a retained fixed-pool run without inferring semantic quality."""
import argparse
import hashlib
import json
import math
from pathlib import Path


def percentile(values, p):
    return sorted(values)[max(0, math.ceil(len(values) * p) - 1)] if values else None


def summarize(root):
    manifest = json.loads((root / 'manifest.json').read_text())
    pools, measurements = [], []
    for path in sorted(root.glob('*/pool.json')):
        pool = json.loads(path.read_text())
        discovery = [json.loads(p.read_text()) for p in sorted(path.parent.glob('discovery-*.json'))]
        pools.append(dict(id=path.parent.name, available=len(pool['candidates']), sufficient=pool['sufficient'],
                          sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                          discovery_process_seconds=sum(r['process_seconds'] for r in discovery),
                          discovery_command_seconds=[r['command_seconds'] for r in discovery],
                          discovery_exit_codes=[r['exit_code'] for r in discovery]))
        for receipt in sorted(path.parent.glob('round-*/receipt.json')):
            record = json.loads(receipt.read_text())
            row = dict(id=path.parent.name, run=receipt.parent.name, **record)
            try:
                payload = json.loads((receipt.parent / 'stdout').read_text())
            except (ValueError, OSError):
                payload = {}
            if not isinstance(payload, dict):
                payload = {}
            row['valid_output'] = isinstance(payload.get('results'), list)
            cap = int(record['argv'][-1])
            expected = [r['url'] for r in pool['candidates'][:cap] if '.pdf' not in r['url'].lower()]
            comparable = (record['exit_code'] == 0 and payload.get('cap') == cap
                          and payload.get('selected_urls') == expected and pool['sufficient'])
            report = payload.get('fetch_report', {})
            row.update(cap=cap, comparable=comparable, command_seconds=payload.get('elapsed_seconds'),
                       fetch_seconds=payload.get('fetch_seconds'), rank_seconds=payload.get('rank_seconds'),
                       accepted=len(pool['candidates']), selected=cap, scheduled=len(payload.get('selected_urls', [])),
                       completed=len(report.get('pages', [])),
                       cancelled=report.get('cancelled'), returned=len(payload.get('results', [])),
                       extracted=sum(x is not None for x in report.get('contents', [])),
                       results=payload.get('results', []))
            measurements.append(row)
    summaries = []
    for cap in (5, 10, 15):
        rows = [r for r in measurements if r['cap'] == cap]
        entry = dict(cap=cap, n=len(rows), comparable=sum(r['comparable'] for r in rows),
                     errors=sum(r['exit_code'] != 0 or not r['valid_output'] for r in rows),
                     valid_empty=sum(r['exit_code'] == 0 and r['valid_output'] and r['returned'] == 0 for r in rows),
                     judged_relevance=None, evidence_coverage=None)
        for field in ('process_seconds', 'command_seconds', 'fetch_seconds', 'rank_seconds'):
            values = [r[field] for r in rows if r[field] is not None]
            entry[field] = dict(n=len(values), p50=percentile(values, .5), p95=percentile(values, .95))
        for field in ('accepted', 'selected', 'scheduled', 'completed', 'cancelled', 'returned', 'extracted'):
            entry[field] = [r[field] for r in rows]
        summaries.append(entry)
    return dict(manifest=manifest, pools=pools, summaries=summaries, measurements=measurements,
                limitations='Three repeats/query; nearest-rank p95 is exploratory. Unknown judgments are null. Discovery excluded from replay. Pool counts and extraction are not relevance. Errors remain in timing samples.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run', type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(args.run), indent=2, ensure_ascii=False))
