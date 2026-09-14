#!/usr/bin/env python3
"""Summarize frozen rankings with explicit, content-keyed assessor judgments."""
import argparse
import hashlib
import json
import math
from pathlib import Path
from evidence_gate import read


def percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)] if values else None


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('study', type=Path)
    p.add_argument('judgments', type=Path)
    args = p.parse_args()
    labels = {(j['query_id'], j['url'], j['content_sha256']): j
              for j in read(args.judgments)['judgments']}
    rows = []
    selection_rows = []
    for q in read(args.study / 'manifest.json')['queries']:
        folder = args.study / q['id']
        if (folder / 'blocked.json').exists():
            rows.append(dict(query_id=q['id'], blocked=read(folder / 'blocked.json')))
            continue
        pre = read(folder / 'selection-pre-rank/stdout')
        selected = next(o['results'][:5] for o in pre['orderings'] if o['policy'] == 'Hybrid')
        js = [labels.get((q['id'], r['url'], hashlib.sha256((r.get('content') or '').encode()).hexdigest())) for r in selected]
        selection_rows.append(dict(query_id=q['id'],
            precision_at_5=sum(j['relevance'] == 2 for j in js)/5 if all(js) else None,
            evidence_at_5=sum(j['evidence'] for j in js)/5 if all(js) else None,
            urls=[r['url'] for r in selected]))
        for round_ in (1, 2):
            fetched = read(folder / f'{round_}-fetch/stdout')
            metadata = read(folder / f'{round_}-metadata/stdout')
            assert fetched['selected_urls'] == metadata['selected_urls'], 'unmatched pool'
            judgments = {}
            for r in fetched['candidates']:
                key = (q['id'], r['url'], hashlib.sha256((r.get('content') or '').encode()).hexdigest())
                judgments[r['url']] = labels.get(key)
            for mode, data in [('metadata', metadata), ('fetch', fetched)]:
                for ordering in data['orderings']:
                    selected = ordering['results'][:5]
                    known = all(judgments[r['url']] is not None for r in selected)
                    report = data['fetch_report'] or {}
                    rows.append(dict(query_id=q['id'], round=round_, mode=mode, policy=ordering['policy'],
                        urls=[r['url'] for r in selected],
                        precision_at_5=sum(judgments[r['url']]['relevance'] == 2 for r in selected)/5 if known else None,
                        evidence_at_5=sum(judgments[r['url']]['evidence'] for r in selected)/5 if known and mode == 'fetch' else 0 if known else None,
                        process_seconds=read(folder / f'{round_}-{mode}/receipt.json')['seconds'],
                        fetch_seconds=data['fetch_seconds'],
                        page_attempts=len(report.get('pages', [])),
                        response_bytes=sum(r.get('response_bytes', 0) for r in report.get('pages', [])),
                        extracted=sum(bool(r.get('content')) for r in data['candidates'])))
    print(json.dumps({'rows': rows, 'separate_pre_rank': selection_rows, 'timing_convention': 'Nearest-rank; process excludes discovery; each process replays all four policies, so timing is shared, not independent per policy.'}, indent=2))


if __name__ == '__main__':
    main()
