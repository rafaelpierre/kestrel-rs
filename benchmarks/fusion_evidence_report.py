#!/usr/bin/env python3
"""Summarize the opt-in live_fusion_latency_evidence test without selecting winning runs."""
import argparse
import collections
import hashlib
import json
import math
from pathlib import Path
import statistics


def median(values):
    return statistics.median(values)


def percentile(values, p):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * p) - 1)]


def escaped(value):
    return str(value).replace('|', '\\|').replace('\n', ' ').replace('[', '\\[').replace(']', '\\]')


def url_link(title, url):
    return f'[{escaped(title)}](<{url}>)'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--input', type=Path, default=Path('benchmarks/results/fusion-evidence/runs.json'))
    parser.add_argument('--output', type=Path, default=Path('docs/fusion-evidence-2026-09-12.md'))
    args = parser.parse_args()
    raw = args.input.read_bytes()
    document = json.loads(raw)
    runs = document['runs']
    quorum = document.get('provider_quorum') or 1
    target = 'five unique results from two providers' if quorum == 2 else 'five unique results, irrespective of provider count'
    queries = list(dict.fromkeys(r['query'] for r in runs))
    assert len(queries) >= 4 and len(runs) == 48, 'Expected the complete four-query repeated experiment'
    grouped = {(r['query'], r['trial'], r['fresh_clients'], r['mode']): r for r in runs}
    assert len(grouped) == len(runs), 'Duplicate run identifiers'
    pairs = [(r, grouped[(r['query'], r['trial'], r['fresh_clients'], 'stream')]) for r in runs if r['mode'] == 'batch']
    per_query = []
    for query in queries:
        selected = [(a, b) for a, b in pairs if a['query'] == query]
        batch = median([a['elapsed_ms'] for a, _ in selected])
        stream = median([b['elapsed_ms'] for _, b in selected])
        per_query.append({'query': query, 'batch_median_ms': batch, 'stream_median_ms': stream,
                          'median_reduction_percent': (batch - stream) / batch * 100,
                          'paired_stream_wins': sum(b['elapsed_ms'] < a['elapsed_ms'] for a, b in selected),
                          'paired_ties': sum(b['elapsed_ms'] == a['elapsed_ms'] for a, b in selected)})
    batch_times = [a['elapsed_ms'] for a, _ in pairs]
    stream_times = [b['elapsed_ms'] for _, b in pairs]
    wins = sum(b < a for a, b in zip(batch_times, stream_times))
    ties = sum(b == a for a, b in zip(batch_times, stream_times))
    summary = {'runs': len(runs), 'pairs': len(pairs), 'threshold_met_runs': sum(r['threshold_met'] for r in runs),
               'batch_median_ms': median(batch_times), 'stream_median_ms': median(stream_times),
               'batch_p95_ms': percentile(batch_times, .95), 'stream_p95_ms': percentile(stream_times, .95),
               'paired_stream_wins': wins, 'paired_ties': ties, 'per_query': per_query}
    summary['overall_median_reduction_percent'] = (summary['batch_median_ms'] - summary['stream_median_ms']) / summary['batch_median_ms'] * 100
    lines = [
        '# Four-query fusion and latency evidence', '',
        f"Live test started **{document['started_utc']}**. All **{summary['threshold_met_runs']}/{len(runs)} runs** reached at least {target}.", '',
        'The comparison is streaming early stopping versus completed-batch early stopping. **Neither arm waits for every provider.** ' + ('These historical measurements used a two-provider quorum before result-minimum precedence was implemented. Current fanout ignores that quorum.' if quorum == 2 else 'The requested provider quorum is ignored; five unique results control stopping.'), '',
        '## Latency', '',
        '| Query | Batch median | Streaming median | Median reduction | Streaming faster in matched pairs |',
        '| --- | ---: | ---: | ---: | ---: |',
    ]
    for row in per_query:
        lines.append(f"| {escaped(row['query'])} | {row['batch_median_ms']:g} ms | {row['stream_median_ms']:g} ms | {row['median_reduction_percent']:.1f}% | {row['paired_stream_wins']}/6 ({row['paired_ties']} ties) |")
    lines += ['', f"Across all 24 observations per arm, median latency changed from **{summary['batch_median_ms']:g} ms to {summary['stream_median_ms']:g} ms** ({summary['overall_median_reduction_percent']:.1f}% lower). Streaming was faster in **{wins}/24** matched pairs, tied in {ties}, and slower in {24-wins-ties}.", '',
              f"Nearest-rank p95 was **{summary['batch_p95_ms']} ms batch / {summary['stream_p95_ms']} ms streaming**. The median improvement does not establish an improvement in tail latency. This is a small, sequential, live-network experiment; provider response variation remains a confounder.", '',
              '| Connection setup | Batch median | Streaming median |', '| --- | ---: | ---: |']
    for fresh in [True, False]:
        a = [r['elapsed_ms'] for r in runs if r['fresh_clients'] == fresh and r['mode'] == 'batch']
        b = [r['elapsed_ms'] for r in runs if r['fresh_clients'] == fresh and r['mode'] == 'stream']
        lines.append(f"| {'Fresh clients' if fresh else 'Reused clients'} | {median(a):g} ms | {median(b):g} ms |")
    lines += ['', '## Final result sets and provider inputs', '',
              'Each example below is the successful streaming run closest to that query’s median streaming latency (ties broken by trial, then connection setup). No example is selected for favorable speed or results. Final results use the actual default provider round-robin fusion and canonical-URL deduplication, followed by taking the first five. Source ranks show the original provider positions.', '']
    for qnum, query in enumerate(queries, 1):
        candidates = [r for r in runs if r['query'] == query and r['mode'] == 'stream' and r['threshold_met']]
        if not candidates:
            lines += [f'### {qnum}. {query}', '', 'No run reached the requested diversity/size threshold; no fused example is claimed.', '']
            continue
        middle = median([r['elapsed_ms'] for r in candidates])
        run = min(candidates, key=lambda r: (abs(r['elapsed_ms'] - middle), r['trial'], r['fresh_clients']))
        observed_sources = collections.defaultdict(set)
        for result in run['results']:
            for source in result['sources']:
                observed_sources[source['engine']].add(source['rank'])
        accepted_inputs = {}
        for engine, ranks in observed_sources.items():
            observed = {r['engine_rank']: r for r in run['providers'][engine]['records']}
            assert ranks <= observed.keys(), f'Missing raw provider records for {query}: {engine}'
            accepted_inputs[engine] = [observed[rank] for rank in sorted(ranks)]
        assert sum(map(len, accepted_inputs.values())) == sum(len(r['sources']) for r in run['results']), 'Source occurrence counts must reconcile'
        final_sources = {s['engine'] for r in run['final_results'] for s in r['sources']}
        assert len(final_sources) >= quorum, 'Final provenance must satisfy the effective stopping contract'
        contributions = ', '.join(f"**{name}: {len(items)}**" for name, items in accepted_inputs.items())
        duplicate_count = sum(map(len, accepted_inputs.values())) - len(run['results'])
        lines += [f'### {qnum}. {query}', '',
                  f"Representative run: trial {run['trial']}, {'fresh' if run['fresh_clients'] else 'reused'} clients, **{run['elapsed_ms']} ms**. Retained provider records: {contributions}; **{len(run['results'])} unique candidates**, {duplicate_count} duplicate occurrences merged. **{run['cancelled']} unfinished requests cancelled.**", '',
                  '| Final position | Result | Provider source / original rank |', '| ---: | --- | --- |']
        for i, result in enumerate(run['final_results'], 1):
            origins = ', '.join(f"{s['engine']} #{s['rank']}" for s in result['sources'])
            lines.append(f"| {i} | {url_link(result['title'], result['url'])} | {origins} |")
        lines += ['', '<details>', '<summary>Retained provider inputs and their positions in the final set</summary>', '']
        for engine, records in accepted_inputs.items():
            lines += [f'**{engine}**', '', '| Provider rank | Original provider record | Final position |', '| ---: | --- | ---: |']
            for record in records:
                positions = [str(i) for i, final in enumerate(run['final_results'], 1) if any(s['engine'] == engine and s['rank'] == record['engine_rank'] for s in final['sources'])]
                lines.append(f"| {record['engine_rank']} | {url_link(record['title'], record['url'])} | {', '.join(positions) or 'Outside final five'} |")
            lines.append('')
        lines += ['</details>', '', '<details>', '<summary>Every provider outcome for this run</summary>', '',
                  '| Provider | Outcome | Reported record count | Contribution retained in fusion |', '| --- | --- | ---: | ---: |']
        for entry in sorted(run['diagnostics'], key=lambda p: p['engine']):
            lines.append(f"| {entry['engine']} | {entry['outcome']} | {entry['result_count']} | {len(accepted_inputs.get(entry['engine'], []))} |")
        lines += ['', '</details>', '']
    outcomes = collections.Counter(p['outcome'] for r in runs for p in r['diagnostics'])
    lines += ['## Relevance observations', '',
              'The raw examples expose a separate quality problem: Bing returned the Rust video game for the ownership query and dictionary definitions of “do” for the solar-panel query. Those are structurally valid search records but off-topic. The report preserves them rather than cleaning the final sets by hand. This experiment demonstrates source fusion and observed latency changes; it does not establish that the final ranking is relevant. Error/challenge rejection alone cannot solve query relevance.', '',
              '## Excluded responses and cancellation evidence', '',
              'Only Bing and Swisscows contributed results in this session. All nine providers were enabled. HTTP failures, known challenges, empty responses and records excluded by URL/site filtering contributed zero. A provider marked `cancelled_min_results` can still contribute completed records received before its body was cancelled; the cancellation itself is not a result.', '',
              '| Logical provider outcome, across all 48 searches | Count |', '| --- | ---: |']
    for name, count in sorted(outcomes.items()):
        lines.append(f'| {name} | {count} |')
    pre_eof = [(r, engine, p) for r in runs if r['mode'] == 'stream' for engine, p in r['providers'].items() if p['first_record_ms'] is not None and p['body_eof_ms'] is None and engine in r['contributing_providers']]
    lines += ['', f'In **{len(pre_eof)} provider responses**, streaming returned complete contributing records while the body had not reached EOF. These are direct observations of stopping during download, rather than merely winning a race between completed provider responses.', '',
              '## Every matched latency pair', '', '| Query | Trial | Clients | Batch ms | Stream ms | Stream minus batch ms | Both thresholds met |', '| --- | ---: | --- | ---: | ---: | ---: | --- |']
    for a, b in pairs:
        lines.append(f"| {escaped(a['query'])} | {a['trial']} | {'Fresh' if a['fresh_clients'] else 'Reused'} | {a['elapsed_ms']} | {b['elapsed_ms']} | {b['elapsed_ms']-a['elapsed_ms']:+} | {'Yes' if a['threshold_met'] and b['threshold_met'] else 'No'} |")
    lines += ['', '## Method and reproduction', '',
              '- Four distinct queries; three repeats; two connection setups; two modes: 48 searches, 24 matched pairs.',
              f'- Both modes require {target}. Both cancel unfinished requests immediately when the effective target is met.',
              '- Concurrency nine; three-second total search deadline; all nine adapters enabled.',
              '- One browser/OS/language profile for the entire experiment. Fresh clients use that same profile. Reused clients have separate pools for each mode; mode order alternates by query and trial.',
              '- Timing covers search and fusion. Page fetching, BM25/body ranking, process startup and client construction are excluded. Results shown are the provider-ranking final five, not an independently reranked set.',
              '- Debug build on one machine/network session. No failed/slow runs are dropped. This demonstrates observed median reductions and correct fusion, not a universal or statistically established speedup.',
              '- Inputs shown above are observed provider records whose original ranks occur in retained fused provenance. Later/unaccepted snapshots and failed-provider records are not presented as accepted inputs.', '',
              f"Profile: `{document['profile']}`.", '', '```sh',
              'cargo test --lib live_fusion_latency_evidence -- --ignored --nocapture',
              'python3 benchmarks/fusion_evidence_report.py --input benchmarks/results/fusion-evidence-result-minimum/runs.json --output docs/fusion-evidence-result-minimum.md', '```', '',
              'The live command tests current result-minimum precedence and writes a separate artifact; it does not reproduce the retired AND-quorum behavior in historical measurements.', '',
              f'Raw observations: `{args.input}`. SHA-256: `{hashlib.sha256(raw).hexdigest()}`.', '',
              'The JSON includes every final set, source record snapshot, provider outcome, cancellation count, and timing observation. The live test is ignored by the normal test suite.']
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text('\n'.join(lines) + '\n')
    args.input.with_name('summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print(f'Report: {args.output}')


if __name__ == '__main__':
    main()
