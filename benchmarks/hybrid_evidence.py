#!/usr/bin/env python3
"""Issue #78 fixed-input ranking and matched live-fetch pilot (no weight tuning)."""
import argparse
import json
import shutil
import time
from pathlib import Path
from evidence_gate import ROOT, execute, git, read, save, sha, source_identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binary, replay = out / 'kestrel', out / 'hybrid_evidence'
    shutil.copy2(ROOT / 'target/release/kestrel', binary)
    shutil.copy2(ROOT / 'target/release/examples/hybrid_evidence', replay)
    music = read(ROOT / 'benchmarks/quality-queries-v1.json')['queries'][:4]
    held = [q for q in read(ROOT / 'benchmarks/codex-search-2026-09-11/queries.json')
            if q['id'] in ('q03', 'q04', 'q07', 'q10')]
    queries = music + held
    flags = ['--no-fetch', '--no-rank', '--top-k', '1000', '--min-results', '20',
             '--search-budget', '5', '--output', 'json']
    save(out / 'manifest.json', dict(revision=git('rev-parse', 'HEAD').decode().strip(),
         sources=source_identity(), binary_sha256=sha(binary), replay_sha256=sha(replay),
         queries=queries, discovery_flags=flags, rounds=2, assessor='Codex GPT-6, unblinded',
         policy='One discovery per query; freeze full artifact candidates. Select first 15. Two rounds of metadata/fetch arms, order reversed in round two. Fetch 2s, request 10s, 2000 chars, 1MB, concurrency/parse 10, cache off. All four ranking policies replay identical candidates. Separate pre-rank selection fetch once. No weights tuned; four canonical queries are held out from music analysis, not unseen agent tasks.',
         statistics='Nearest-rank p50/p95; exploratory paired observations, not reliable population tails. Discovery excluded from paired process time; upstream caches uncontrolled.'))
    for q in queries:
        folder = out / q['id']
        folder.mkdir()
        artifacts = folder / 'artifacts'
        receipt = execute(folder / 'discovery', [str(binary), 'search', q['query'], *flags],
            extra_env={'KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR': str(artifacts),
                       'KESTRELSEARCH_BENCHMARK_RUN_ID': q['id'],
                       'KESTRELSEARCH_PROVIDER_TRACE_DIR': str(folder / 'traces')})
        paths = list(artifacts.glob('*.json'))
        if receipt['exit_code'] or len(paths) != 1:
            save(folder / 'blocked.json', {'reason': 'discovery failed or missing/ambiguous full artifact'})
            continue
        pool = {'queries': [q['query']], 'candidates': read(paths[0])['candidates']}
        save(folder / 'pool.json', pool)
        for round_, modes in enumerate((('metadata', 'fetch'), ('fetch', 'metadata')), 1):
            for mode in modes:
                execute(folder / f'{round_}-{mode}', [str(replay), str(folder / 'pool.json'), mode, 'provider'])
                time.sleep(.25)
        execute(folder / 'selection-pre-rank', [str(replay), str(folder / 'pool.json'), 'fetch', 'pre-rank'])
        print(q['id'], len(pool['candidates']), flush=True)


if __name__ == '__main__':
    main()
