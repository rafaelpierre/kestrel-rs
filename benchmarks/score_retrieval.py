#!/usr/bin/env python3
"""Score a saved batch with explicit judgments; never substitutes keyword matching."""
import argparse
import collections
import json
from pathlib import Path
import random
import statistics
from urllib.parse import urlparse


def summarize(runs, judgments, domains):
    unknown = sorted({(r['query_id'], a['url']) for r in runs for a in r['results']
                      if (r['query_id'], a['url']) not in judgments})
    scores = [[judgments.get((r['query_id'], a['url'])) for a in r['results']] for r in runs]
    restricted = [r for r in runs if domains.get(r['query_id'])]
    returned = sum(len(r['results']) for r in restricted)
    compliant = sum((urlparse(a['url']).hostname or '').lower().rstrip('.') == domains[r['query_id']]
                    or (urlparse(a['url']).hostname or '').lower().rstrip('.').endswith('.' + domains[r['query_id']])
                    for r in restricted for a in r['results'])
    result = {'runs': len(runs), 'unjudged_pairs': unknown,
              'precision_at_5': None if unknown else sum(s == 2 for ss in scores for s in ss)/(5*len(runs)),
              'graded_relevance': None if unknown else sum(sum(ss) for ss in scores)/(10*len(runs)),
              'runs_with_direct_hit': None if unknown else sum(2 in ss for ss in scores),
              'returned_domain_compliance': compliant/returned if returned else None,
              'domain_slot_coverage': compliant/(5*len(restricted)) if restricted else None,
              'duplicate_slots': sum(len(r['results']) - len({a['url'] for a in r['results']}) for r in runs)}
    if not unknown:
        groups = collections.defaultdict(list)
        for r, ss in zip(runs, scores):
            groups[r['query_id']].append(sum(s == 2 for s in ss)/5)
        means = [statistics.mean(v) for v in groups.values()]
        rng = random.Random(20260911)
        boot = sorted(statistics.mean(rng.choices(means, k=len(means))) for _ in range(2000))
        result['query_bootstrap_95_percent_interval'] = [boot[49], boot[1949]]
        result['per_query_precision_at_5'] = {q:statistics.mean(v) for q,v in groups.items()}
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('runs', type=Path)
    p.add_argument('judgments', type=Path)
    p.add_argument('--queries', type=Path, default=Path(__file__).parent/'codex-search-2026-09-11/queries.json')
    args = p.parse_args()
    data = json.loads(args.queries.read_text())
    qs = data if isinstance(data, list) else data['queries']
    domains = {q['id']:q['domain'].lower().rstrip('.') for q in qs if q.get('domain')}
    js = json.loads(args.judgments.read_text())['judgments']
    judgments = {(j['query_id'],j['url']):j['score'] for j in js}
    if len(judgments) != len(js) or any(s not in [0,1,2] for s in judgments.values()):
        p.error('judgments must have unique query/URL pairs and scores 0, 1, or 2')
    runs = list(map(json.loads, args.runs.read_text().splitlines()))
    groups = collections.defaultdict(list)
    for r in runs:
        groups[r['condition']].append(r)
    print(json.dumps({c:summarize(rs,judgments,domains) for c,rs in groups.items()},indent=2))


if __name__ == '__main__':
    main()
