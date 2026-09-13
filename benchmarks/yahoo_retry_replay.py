#!/usr/bin/env python3
"""Offline prefix replay of diagnostic attempts; does not predict future traffic."""
import argparse
import json
from pathlib import Path


def replay(record, cap):
    attempts = record['attempts']
    retained = attempts[:cap]
    ids = {a['attempt_id'] for a in retained}
    elapsed = 0
    censored = False
    last_attempt = None
    for interval in record['intervals']:
        if interval['attempt_id'] is not None:
            last_attempt = interval['attempt_id']
        if last_attempt not in ids:
            continue
        # Backoff has a null attempt ID in schema 1, so use sequence order.
        if (interval['phase'] == 'backoff' and len(retained) == cap
                and last_attempt == retained[-1]['attempt_id']):
            break
        elapsed += interval['elapsed_ms']
        censored |= interval['censored']
    # HTTP recovery is distinct from valid provider results.
    recovered = any(a.get('http_status') is not None and 200 <= a['http_status'] < 300
                    and a.get('outcome') == 'response' for a in retained)
    return dict(cap=cap, sends=len(retained), sends_saved=len(attempts)-len(retained),
                http_recovered=recovered, retained_attempt_ms=elapsed, censored=censored)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    args = p.parse_args()
    rows = []
    for path in sorted(args.directory.glob('*/traces/outcome-*.json')):
        data = json.loads(path.read_text())
        lifecycle = data['lifecycle']
        rows.append(dict(source=str(path), logical_outcome=lifecycle['logical_outcome'],
                         observed_ms=data.get('elapsed_ms'),
                         policies=[replay(lifecycle, cap) for cap in (1, 2, 3)]))
    print(json.dumps(dict(method='Counterfactual prefix replay, excluding queue and startup. '
                         'Not an executed retry policy, cooldown or causal wall-time estimate. '
                         'Successful HTTP responses may be empty or invalid.', rows=rows), indent=2))


if __name__ == '__main__':
    main()
