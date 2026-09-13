#!/usr/bin/env python3
"""Summarize frozen replay separately from live command timings, without page text."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import statistics

from quality_report import judgment_key, load_judgments, quantile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--replay', type=Path, required=True)
    p.add_argument('--batch', type=Path, required=True)
    p.add_argument('--judgments', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    source = {r['run_id']: r for r in map(json.loads, (args.batch/'runs.jsonl').read_text().splitlines())}
    judgments = load_judgments(args.judgments)
    records, groups, inputs, missing = [], defaultdict(list), {}, []
    for row in map(json.loads, args.replay.read_text().splitlines()):
        if row.get('replay_status'):
            missing.append(row)
            continue
        source_run = source[row['source_run_id']]
        if source_run.get('effective', {}).get('top_k', 5) != 5:
            raise ValueError('rank_replay example returns fixed top five; source top-k must be five')
        record = {k: row[k] for k in ['source_run_id', 'query_id', 'round', 'condition', 'input_sha256',
                                      'replay_binary_sha256', 'candidate_count', 'retained_count']}
        record.update(source_condition=source_run['condition'], results=[])
        for result in row['results']:
            key = judgment_key(row['query_id'], result)
            j = judgments.get(key, {})
            record['results'].append(dict(url=result['url'], content_sha256=key[2],
                                         relevance=j.get('relevance'), evidence=j.get('evidence')))
        records.append(record)
        groups[(source_run['condition'], row['condition'])].append(record)
        previous = inputs.get(row['source_run_id'])
        if previous and previous['input_sha256'] != row['input_sha256']:
            raise ValueError('policies did not receive identical frozen inputs')
        inputs[row['source_run_id']] = dict(input_sha256=row['input_sha256'],
                                            all_policies_process_seconds=row['all_policies_process_seconds'])
    summary = {}
    for (source_condition, policy), rs in groups.items():
        scores = [x.get('relevance') for r in rs for x in r['results']]
        evidence = [x.get('evidence') for r in rs for x in r['results']]
        summary.setdefault(source_condition, {})[policy] = dict(runs=len(rs),
            returned_slots=sum(len(r['results']) for r in rs),
            precision_at_5=None if None in scores else sum(s == 2 for s in scores)/(5*len(rs)),
            page_evidence_slot_coverage=None if None in evidence else sum(evidence)/(5*len(rs)))
    times = [r['all_policies_process_seconds'] for r in inputs.values()]
    output = dict(schema_version=1, note='All five policies run in one process per frozen input. No per-policy command timing is measured. Quality comparisons are paired within each input; separate the source fetching treatments.',
                  replay_sha256=hashlib.sha256(args.replay.read_bytes()).hexdigest(),
                  judgments_sha256=hashlib.sha256(args.judgments.read_bytes()).hexdigest(),
                  inputs=len(inputs), all_policies_process_seconds=dict(n=len(times),
                    p50=statistics.median(times) if times else None, p95=quantile(times, .95)),
                  missing=missing, summary=summary, records=records)
    with args.output.open('x') as stream:
        json.dump(output, stream, indent=2)
        stream.write('\n')


if __name__ == '__main__':
    main()
