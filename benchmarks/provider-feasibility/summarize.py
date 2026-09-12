#!/usr/bin/env python3
"""Summarize sanitized observations with explicit attempt/quality denominators."""
import collections
import json
import math
from pathlib import Path
import statistics
import sys

root = Path(__file__).parent
rows = [json.loads(line) for path in sys.argv[1:] for line in Path(path).read_text().splitlines()]
judgments = {(j['query_id'], j['url']): j for j in json.loads((root / 'judgments.json').read_text())}


def percentiles(values):
    values = sorted(values)
    return None if not values else {'n': len(values), 'p50': statistics.median(values), 'p95_nearest_rank': values[math.ceil(.95 * len(values)) - 1]}


summary = {}
for provider in sorted({r['provider'] for r in rows}):
    subset = [r for r in rows if r['provider'] == provider]
    attempts = [r for r in subset if r['outcome'] != 'skipped_retry_after']
    success = [r for r in attempts if r['outcome'] == 'nonempty']
    judged = [r for r in subset if r['query_id'] != 'punctuation-unicode']
    precision, conditional, usable = [], [], 0
    for r in judged:
        top = r.get('results', [])[:5]
        relevant = sum(judgments[(r['query_id'], result['url'])]['relevant'] for result in top)
        precision.append(relevant / 5)
        if top:
            conditional.append(relevant / 5)
        usable += relevant > 0
    summary[provider] = {
        'scheduled': len(subset), 'logical_requests_attempted': len(attempts),
        'skipped': len(subset) - len(attempts),
        'outcomes': dict(collections.Counter(r['outcome'] for r in subset)),
        'statuses': dict(collections.Counter(str(r.get('status', 'not_captured')) for r in attempts)),
        'nonempty': len(success),
        'success_seconds': percentiles([r['elapsed_seconds'] for r in success]),
        'other_seconds': percentiles([r['elapsed_seconds'] for r in attempts if r not in success]),
        'quality_judged_scheduled': len(judged), 'usable_searches': usable,
        'p5_scheduled': statistics.mean(precision),
        'p5_conditional_nonempty': statistics.mean(conditional) if conditional else None,
        'by_session': {session: percentiles([r['elapsed_seconds'] for r in attempts if r['session'] == session]) for session in ['fresh_client', 'reused_client']},
    }
print(json.dumps(summary, indent=2))
