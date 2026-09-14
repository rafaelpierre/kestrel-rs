#!/usr/bin/env python3
"""Controlled body interventions; always call the production Rust replay ranker."""
import argparse
import copy
from pathlib import Path
from evidence_gate import execute, read, save


def variants(candidates):
    for name in ('original', 'missing', 'truncated-200', 'boilerplate'):
        rows = copy.deepcopy(candidates)
        for row in rows:
            body = row.get('content')
            if name == 'missing':
                row['content'] = None
            elif body and name != 'original':
                text = body.split('\n\n', 1)[-1] if body.startswith('Source: ') else body
                text = text[:200] if name == 'truncated-200' else 'Menu Privacy Cookies Subscribe ' * 50
                row['content'] = f"Source: {row['url']}\n\n{text}"
        yield name, rows


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('study', type=Path)
    p.add_argument('output', type=Path)
    args = p.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    replay = str((args.study / 'hybrid_evidence').resolve())
    for q in read(args.study / 'manifest.json')['queries']:
        source = args.study / q['id'] / '1-fetch/stdout'
        if not source.exists():
            continue
        captured = read(source)
        for name, rows in variants(captured['candidates']):
            path = args.output / f"{q['id']}-{name}.json"
            save(path, dict(queries=captured['queries'], candidates=rows))
            execute(args.output / f"{q['id']}-{name}", [replay, str(path.resolve()), 'replay', 'provider'])
    # Known relevance, constant metadata and corpus size; only one body changes.
    for size in (1, 2, 3):
        for multi in (False, True):
            queries = ['rust ownership', 'python cancellation'] if multi else ['rust ownership']
            rows = [dict(title='rust ownership', url=f'https://example.test/{i}', snippet='guide',
                         display_url='', query=queries[i % len(queries)], content=None) for i in range(size)]
            for name, body in [('missing', None), ('useful', 'Rust ownership moves values. Borrow a reference to retain ownership.'),
                               ('boilerplate', 'Menu Privacy Cookies Subscribe ' * 50),
                               ('truncated', 'Rust ownership')]:
                sample = copy.deepcopy(rows)
                sample[0]['content'] = body
                slug = f'synthetic-{size}-{multi}-{name}'
                path = args.output / f'{slug}.json'
                save(path, dict(queries=queries, candidates=sample))
                execute(args.output / slug, [replay, str(path.resolve()), 'replay', 'provider'])


if __name__ == '__main__':
    main()
