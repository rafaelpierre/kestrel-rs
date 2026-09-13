"""Summaries retain failures, unknown judgments and query-cluster uncertainty."""
import hashlib
import json
import math
import random
import statistics
from collections import defaultdict


def digest_content(result):
    return hashlib.sha256((result.get('content') or '').encode()).hexdigest()


def judgment_key(query_id, result):
    return (query_id, result['url'], digest_content(result))


def quantile(values, p):
    return sorted(values)[max(0, math.ceil(p * len(values)) - 1)] if values else None


def interval_by_query(runs, field, statistic):
    groups = defaultdict(list)
    for run in runs:
        if run.get(field) is not None:
            groups[run['query_id']].append(run[field])
    if len(groups) < 2:
        return None
    clusters = list(groups.values())
    rng = random.Random(75)
    samples = [statistic([v for cluster in rng.choices(clusters, k=len(clusters)) for v in cluster])
               for _ in range(1000)]
    return [quantile(samples, .025), quantile(samples, .975)]


def distribution(runs, field):
    values = [r[field] for r in runs if r.get(field) is not None]
    return dict(n=len(values), missing=len(runs)-len(values), p50=statistics.median(values) if values else None,
                p95=quantile(values, .95), p50_query_bootstrap_95_interval=interval_by_query(runs, field, statistics.median),
                p95_query_bootstrap_95_interval=interval_by_query(runs, field, lambda v: quantile(v, .95)),
                tail_note='Observed nearest-rank p95; exploratory, especially with fewer than 100 runs or few independent queries. Repeats share network/provider state.')


def pool_check(run, required):
    artifacts = run.get('artifacts', [])
    if len(artifacts) != 1:
        return dict(sufficient=None, required=required, reason='missing or ambiguous candidate artifact')
    counts = artifacts[0].get('candidate_counts') or {}
    available = counts.get('after_search')
    candidates = artifacts[0].get('candidates', [])
    return dict(required=required, available=available, selected=counts.get('after_candidate_selection'),
                sufficient=available >= required if available is not None else None,
                selected_url_sha256=hashlib.sha256(json.dumps([r['url'] for r in candidates]).encode()).hexdigest())


def load_judgments(path):
    if path is None:
        return {}
    rows = json.loads(path.read_text())['judgments']
    result = {}
    for row in rows:
        key = (row['query_id'], row['url'], row['content_sha256'])
        if key in result or row['relevance'] not in [None, 0, 1, 2] or row['evidence'] not in [None, 0, 1]:
            raise ValueError('judgments require unique query/URL/content keys, relevance 0/1/2 and evidence 0/1 (or null)')
        if (row['relevance'] is not None or row['evidence'] is not None) and not row.get('rationale'):
            raise ValueError('judged results require a rationale')
        result[key] = row
    return result


def judgment_template(runs):
    rows = {}
    for run in runs:
        for r in run['results']:
            key = judgment_key(run['query_id'], r)
            rows[key] = dict(query_id=key[0], url=key[1], content_sha256=key[2], title=r.get('title'),
                             snippet=r.get('snippet'), content=r.get('content'), relevance=None, evidence=None, rationale='')
    return dict(schema_version=1, rubric='Relevance: 0 off-topic, 1 useful background, 2 directly addresses query intent. Evidence: 1 only when captured page content supports that intent, otherwise 0. Null means unjudged; do not infer evidence from successful fetching.',
                judgments=list(rows.values()))


def summarize(runs, judgments=None):
    judgments = judgments or {}
    groups = defaultdict(list)
    for r in runs:
        groups[r['condition']].append(r)
    summary = {}
    for name, rs in groups.items():
        judged = [judgments.get(judgment_key(r['query_id'], result), {}) for r in rs for result in r['results']]
        unknown_relevance = sum(j.get('relevance') is None for j in judged)
        unknown_evidence = sum(j.get('evidence') is None for j in judged)
        expected = sum(r.get('effective', {}).get('top_k', 5) for r in rs)
        counts = [a.get('candidate_counts') or {} for r in rs for a in r.get('artifacts', [])]
        providers = [p for r in rs for a in r.get('artifacts', []) for p in a.get('diagnostics', {}).get('providers', [])]
        fetches = [a['diagnostics']['fetch'] for r in rs for a in r.get('artifacts', []) if a.get('diagnostics', {}).get('fetch')]
        summary[name] = dict(runs=len(rs), queries=len({r['query_id'] for r in rs}),
            process_seconds=distribution(rs, 'process_seconds'), command_seconds=distribution(rs, 'command_seconds'),
            timeouts=sum(r['exit_code'] == 124 for r in rs), errors=sum(r['exit_code'] != 0 for r in rs),
            empty_rate=sum(not r['results'] for r in rs)/len(rs),
            valid_empty_rate=sum(not r['results'] and r['exit_code'] == 0 for r in rs)/len(rs),
            returned_slots=sum(len(r['results']) for r in rs), expected_slots=expected,
            candidate_counts=dict(observations=len(counts), sums={key: sum(c.get(key, 0) for c in counts)
                for key in ['after_search', 'after_candidate_selection', 'with_content', 'returned']}),
            provider_counts=dict(observations=len(providers), sums={key: sum(p.get(key, 0) for p in providers) for key in ['raw_result_count', 'filtered_count', 'result_count']},
                                 note='Provider accepted counts can overlap across providers; after_search is deduplicated. Missing artifacts are unknown.'),
            fetch_counts=dict(observations=len(fetches), sums={key: sum(f.get(key, 0) for f in fetches) for key in ['cache_hits', 'cache_misses', 'response_bytes', 'cancelled']}),
            insufficient_pools=sum(r.get('pool_check', {}).get('sufficient') is False for r in rs),
            unknown_pools=sum(r.get('pool_check', {}).get('sufficient') is None for r in rs),
            unjudged_relevance=unknown_relevance, unjudged_evidence=unknown_evidence,
            precision_at_k=None if unknown_relevance else sum(j['relevance'] == 2 for j in judged)/expected,
            evidence_slot_coverage=None if unknown_evidence else sum(j['evidence'] for j in judged)/expected)
    return summary


def compare_inputs(runs, stage):
    groups = defaultdict(list)
    for run in runs:
        groups[(run['query_id'], run['round'])].append(run)
    comparisons = []
    for (query, round_), peers in groups.items():
        pools = []
        for run in peers:
            artifacts = run.get('artifacts', [])
            if len(artifacts) != 1 or run['exit_code'] != 0:
                pools.append(None)
            else:
                pools.append([{k: r.get(k) for k in ['url', 'title', 'snippet', 'engine', 'engine_rank', 'sources']}
                              for r in artifacts[0].get('candidates', [])])
        known = len(peers) > 1 and all(pool for pool in pools)
        metadata_match = None
        if known:
            largest = max(pools, key=len)
            metadata_match = all(pool == (largest[:len(pool)] if stage == 'candidates' else largest) for pool in pools)
        comparisons.append(dict(query_id=query, round=round_, conditions=[r['condition'] for r in peers],
                                selected_metadata_inputs_match=metadata_match,
                                sufficient_pools=all(r.get('pool_check', {}).get('sufficient') is True for r in peers)))
    return dict(note='Live matching is a diagnostic, not randomization or a guarantee of fixed inputs. Candidate sweeps compare prefixes of the largest selected pool. Ranking isolation requires frozen replay including identical content.',
                pairs=comparisons)


def main():
    import argparse
    from pathlib import Path
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('batch', type=Path)
    parser.add_argument('--judgments', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    runs = list(map(json.loads, (args.batch/'runs.jsonl').read_text().splitlines()))
    metadata = json.loads((args.batch/'metadata.json').read_text())
    judgments = load_judgments(args.judgments)
    result = dict(metadata=metadata, summary=summarize(runs, judgments),
                  input_comparisons=compare_inputs(runs, metadata['stage']),
                  per_query={q: summarize([r for r in runs if r['query_id'] == q], judgments) for q in metadata['query_ids']},
                  judgments_sha256=hashlib.sha256(args.judgments.read_bytes()).hexdigest() if args.judgments else None)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')


if __name__ == '__main__':
    main()
