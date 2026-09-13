#!/usr/bin/env python3
"""Export compact, content-free measurements and judgments for a reproducible report."""
import argparse
import hashlib
import json
from pathlib import Path

from quality_report import compare_inputs, judgment_key, load_judgments, summarize


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--pilot', type=Path, required=True)
    p.add_argument('--judgments', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    if args.output.exists():
        p.error('export destination must be new')
    judgments = load_judgments(args.judgments)
    args.output.mkdir(parents=True)
    metadata, summaries, comparisons, per_query = {}, {}, {}, {}
    with (args.output/'measurements.jsonl').open('x') as stream:
        for batch in sorted(args.pilot.glob('*/metadata.json')):
            stage = batch.parent.name
            meta = json.loads(batch.read_text())
            runs = list(map(json.loads, (batch.parent/'runs.jsonl').read_text().splitlines()))
            # Keep source/binary/environment hashes; remove machine-specific directory paths.
            meta.pop('binary', None)
            meta.pop('query_file', None)
            metadata[stage] = meta
            summaries[stage] = summarize(runs, judgments)
            comparisons[stage] = compare_inputs(runs, stage)
            per_query[stage] = {q: summarize([r for r in runs if r['query_id'] == q], judgments) for q in sorted({r['query_id'] for r in runs})}
            for run in runs:
                record = {key: run[key] for key in ['run_id', 'query_id', 'round', 'condition', 'effective',
                                                  'started_at', 'process_seconds', 'command_seconds', 'exit_code', 'pool_check']}
                record.update(stage=stage, results=[])
                # The measured argv is retained with just its frozen-binary path normalized.
                record['command'] = ['./target/release/kestrel', *run['command'][1:]]
                for result in run['results']:
                    key = judgment_key(run['query_id'], result)
                    judgment = judgments.get(key, {})
                    record['results'].append(dict(url=result['url'], content_sha256=key[2],
                        relevance=judgment.get('relevance'), evidence=judgment.get('evidence')))
                stream.write(json.dumps(record)+'\n')
    for filename, data in [('metadata.json', metadata), ('summary.json', summaries), ('input-comparisons.json', comparisons), ('per-query.json', per_query)]:
        (args.output/filename).write_text(json.dumps(data, indent=2)+'\n')
    (args.output/'judgments.json').write_bytes(args.judgments.read_bytes())
    (args.output/'manifest.json').write_text(json.dumps({p.name: hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(args.output.iterdir())}, indent=2)+'\n')


if __name__ == '__main__':
    main()
