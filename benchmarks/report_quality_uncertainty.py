#!/usr/bin/env python3
"""Exploratory paired query-cluster intervals for exported quality and empty rates."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import random

from quality_report import quantile


def intervals(runs):
    by_query = defaultdict(lambda: defaultdict(list))
    for run in runs:
        by_query[run['query_id']][run['condition']].append(run)
    conditions = sorted({r['condition'] for r in runs})
    samples = defaultdict(list)
    queries = list(by_query.values())
    if len(queries) < 2:
        return None
    rng = random.Random(75)
    for _ in range(1000):
        draw = rng.choices(queries, k=len(queries))
        for condition in conditions:
            rs = [r for q in draw for r in q[condition]]
            if not rs:
                continue
            slots = sum(r['effective']['top_k'] for r in rs)
            samples[(condition, 'empty_rate')].append(sum(not r['results'] for r in rs)/len(rs))
            for key, field in [('precision_at_k', 'relevance'), ('page_evidence_slot_coverage', 'evidence')]:
                scores = [x[field] for r in rs for x in r['results']]
                if None not in scores:
                    samples[(condition, key)].append(sum(s == (2 if field == 'relevance' else 1) for s in scores)/slots)
    output = {c: {k: [quantile(samples[(c,k)], .025), quantile(samples[(c,k)], .975)]
                  if len(samples[(c,k)]) == 1000 else None
                  for k in ['empty_rate', 'precision_at_k', 'page_evidence_slot_coverage']} for c in conditions}
    if 'search-default-5s' in conditions:
        baseline = samples[('search-default-5s', 'precision_at_k')]
        for c in conditions:
            values = samples[(c, 'precision_at_k')]
            output[c]['precision_difference_from_default_5s'] = (
                [quantile([a-b for a,b in zip(values,baseline)], p) for p in [.025,.975]]
                if len(values) == len(baseline) == 1000 else None)
    return output


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('measurements', type=Path)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    stages = defaultdict(list)
    for run in map(json.loads,args.measurements.read_text().splitlines()):
        stages[run['stage']].append(run)
    output = dict(method='1000 paired query-cluster bootstrap resamples, seed 75, percentile 95% intervals. Seven clusters and shared network state limit inference; no independent-trial or reliable-tail claim.',
                  intervals={stage: intervals(runs) for stage,runs in stages.items()})
    with args.output.open('x') as stream:
        json.dump(output,stream,indent=2)
        stream.write('\n')


if __name__ == '__main__':
    main()
