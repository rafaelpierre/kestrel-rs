#!/usr/bin/env python3
"""Append-only, assessor-driven canonical evidence gate. See evidence-gate.md."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
DATASET = 'benchmarks/codex-search-2026-09-11/queries.json'
POLICY = {
    'search_flags': ['--no-fetch', '--no-rank', '-k', '20', '--min-results', '20',
                     '--search-budget', '10', '--output', 'json'],
    'fetch_flags': ['--output', 'json', '--timeout', '15', '--content-limit',
                    '100000', '--max-response-bytes', '4000000'],
    'process_timeout_seconds': 30, 'max_searches': 2, 'max_fetches': 3,
    'cache': 'disabled; no page cache flags; upstream caches uncontrolled',
    'recovery': 'One keyword FTS refinement or unchanged-query retry after inspecting all candidates; preserve intent, domain, entity, version and date; record reason before execution.',
    'selection': 'Assessor inspects every returned title, URL and snippet; chooses relevant primary sources. Fetch only discovered URLs. Stop when answer minima are supported, otherwise use remaining bounded recovery.',
    'assessment': 'Manual semantic assessment against frozen AGENTS.md q01-q10 minima. Missing judgments, partial answers and abstentions do not pass. Passage matching checks provenance, not truth or sufficiency.',
    'timing': 'Sum subprocess monotonic wall seconds by search/fetch, including failed/timeout attempts; not agent end-to-end latency. No percentile/causal claims.',
    'redirects': 'Fetch follows CLI redirects. CLI does not expose redirect chains; do not infer official migration from successful extraction. A required undocumented redirect remains a blocker.',
}
LAYERS = {'upstream', 'constraints', 'collection', 'selection', 'extraction', 'assessment', 'none'}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    # Exclusive creation: prior runs and judgments are never silently replaced.
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write('\n')


def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT), *args])


def source_identity():
    names = git('ls-files', '-z', '--cached', '--others', '--exclude-standard').decode().split('\0')
    files = {}
    for name in sorted(set(filter(None, names))):
        path = ROOT / name
        if path.is_file():
            files[name] = {'sha256': sha(path), 'mode': oct(path.stat().st_mode & 0o777)}
        elif path.is_symlink():
            files[name] = {'symlink': os.readlink(path)}
        else:
            files[name] = {'missing': True}
    return files


def execute(directory, argv, timeout=30, cwd=ROOT, extra_env=None, decision=None):
    directory.mkdir(parents=True, exist_ok=False)
    started = datetime.now(timezone.utc).isoformat()
    save(directory / 'request.json', {'argv': argv, 'started_at': started, 'timeout_seconds': timeout, 'decision': decision})
    env = os.environ.copy()
    # Keep payloads local even when the user's shell config enables remote tracing.
    env['KESTRELSEARCH_OTEL_ENABLED'] = 'false'
    for key in ('KESTRELSEARCH_PROVIDER_TRACE_DIR', 'KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR',
                'KESTRELSEARCH_BENCHMARK_RUN_ID'):
        env.pop(key, None)
    env.update(extra_env or {})
    start = time.monotonic()
    with (directory / 'stdout').open('wb') as out, (directory / 'stderr').open('wb') as err:
        try:
            result = subprocess.run(argv, cwd=cwd, env=env, stdout=out, stderr=err, timeout=timeout)
            code, timed_out = result.returncode, False
        except subprocess.TimeoutExpired:
            code, timed_out = 124, True
        except OSError as error:
            err.write(str(error).encode())
            code, timed_out = 127, False
    receipt = {'argv': argv, 'started_at': started, 'ended_at': datetime.now(timezone.utc).isoformat(),
               'seconds': time.monotonic() - start, 'exit_code': code, 'timed_out': timed_out,
               'stdout_sha256': sha(directory / 'stdout'), 'stderr_sha256': sha(directory / 'stderr')}
    save(directory / 'receipt.json', receipt)
    return receipt


def canonical_questions(path):
    questions = read(path)
    if [row['id'] for row in questions] != [f'q{i:02d}' for i in range(1, 11)]:
        raise ValueError('Dataset must contain exactly q01-q10 in canonical order')
    return questions


def initialize(run, assessor):
    canonical_questions(ROOT / DATASET)
    run.mkdir(parents=True, exist_ok=False)
    before = source_identity()
    build = execute(run / 'build', ['cargo', 'build', '--release', '--locked'], timeout=900)
    if build['exit_code'] or before != source_identity():
        raise ValueError('Build failed or source changed during build; preserve this directory and start a new run')
    binary = ROOT / 'target/release/kestrel'
    skill_project = run / 'skill-project'
    skill_project.mkdir()
    install = execute(run / 'skill-install', [str(binary), 'skill', 'install', '--agent', 'codex', '--scope', 'project'], cwd=skill_project)
    if install['exit_code']:
        raise ValueError('Skill installation failed')
    skill = skill_project / '.codex/skills/kestrelsearch/SKILL.md'
    version = execute(run / 'version', [str(binary), '--version'])
    if version['exit_code']:
        raise ValueError('Version check failed')
    (run / 'AGENTS.md').write_bytes((ROOT / 'AGENTS.md').read_bytes())
    (run / 'queries.json').write_bytes((ROOT / DATASET).read_bytes())
    (run / 'SKILL.md').write_bytes(skill.read_bytes())
    save(run / 'manifest.json', {'schema_version': 1, 'assessor': assessor, 'policy': POLICY,
         'revision': git('rev-parse', 'HEAD').decode().strip(), 'head_tree': git('rev-parse', 'HEAD^{tree}').decode().strip(),
         'source_files': before, 'binary': str(binary), 'binary_sha256': sha(binary),
         'version': (run / 'version/stdout').read_text().strip(), 'dataset_sha256': sha(run / 'queries.json'),
         'skill_sha256': sha(run / 'SKILL.md'), 'rubric_sha256': sha(run / 'AGENTS.md'),
         'telemetry': 'remote export disabled; local provider traces and benchmark artifacts enabled per search'})


def check(run):
    manifest = read(run / 'manifest.json')
    if (manifest['source_files'] != source_identity() or manifest['binary_sha256'] != sha(manifest['binary'])
            or manifest['policy'] != POLICY or manifest['dataset_sha256'] != sha(run / 'queries.json')
            or manifest['skill_sha256'] != sha(run / 'SKILL.md') or manifest['rubric_sha256'] != sha(run / 'AGENTS.md')):
        raise ValueError('Frozen inputs changed; start a separately identified complete run')
    return manifest


def attempts(run, qid, kind):
    return sorted(path for path in (run / qid).glob(f'{kind}-*') if path.is_dir())


def payload(attempt):
    receipt = read(attempt / 'receipt.json')
    if receipt['exit_code']:
        return {}
    if sha(attempt / 'stdout') != receipt['stdout_sha256']:
        raise ValueError('Captured stdout changed')
    return read(attempt / 'stdout')


def discovered(run, qid):
    return {row['url'] for attempt in attempts(run, qid, 'search')
            for row in payload(attempt).get('results', [])}


def retrieve(run, qid, kind, value=None, reason=None):
    manifest = check(run)
    questions = {row['id']: row for row in read(run / 'queries.json')}
    if qid not in questions:
        raise ValueError('Unknown question ID')
    if (run / qid / 'assessment.json').exists():
        raise ValueError('Question already assessed; start a new complete run')
    previous = attempts(run, qid, kind)
    limit = POLICY['max_searches' if kind == 'search' else 'max_fetches']
    if len(previous) >= limit:
        raise ValueError(f'{kind} budget exhausted (interrupted attempts count)')
    if kind == 'search':
        query = questions[qid]['query']
        if not previous and value is not None:
            raise ValueError('Initial search must use exact manifest query')
        if previous:
            if not reason or not value:
                raise ValueError('Recovery needs a query and reason')
            query = value
            domain = questions[qid].get('domain')
            if domain and f'site:{domain}' not in query.split():
                raise ValueError('Recovery must retain the manifest site restriction')
        value = query
    elif not reason or value not in discovered(run, qid):
        raise ValueError('Fetch needs a selection reason and a URL returned in this question/run')
    directory = run / qid / f'{kind}-{len(previous) + 1}'
    # execute reserves the attempt before writing the decision or launching a process.
    decision = {'value': value, 'reason': reason or 'Exact manifest query', 'intent': questions[qid]['intent']}
    flags = POLICY['search_flags' if kind == 'search' else 'fetch_flags']
    result = execute(directory, [manifest['binary'], kind, value, *flags], decision=decision, extra_env={
        'KESTRELSEARCH_PROVIDER_TRACE_DIR': str(directory / 'traces'),
        'KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR': str(directory / 'artifacts'),
        'KESTRELSEARCH_BENCHMARK_RUN_ID': f'{qid}-{directory.name}'})
    print(json.dumps({'id': qid, 'attempt': str(directory), **result}))


def validate_assessment(run, qid, judgment):
    if judgment.get('verdict') not in {'pass', 'fail'} or judgment.get('layer') not in LAYERS:
        raise ValueError('Need pass/fail verdict and a recognized failure layer')
    for key in ('answer', 'rationale', 'inspection', 'fetch_disposition'):
        if not isinstance(judgment.get(key), str) or not judgment[key].strip():
            raise ValueError(f'Missing {key}')
    if not attempts(run, qid, 'search'):
        raise ValueError('Question has not been run')
    citations = judgment.get('citations', [])
    if judgment['verdict'] == 'pass' and not citations:
        raise ValueError('Pass requires supporting evidence')
    for citation in citations:
        name = citation['attempt']
        allowed = {path.name: path for kind in ('search', 'fetch') for path in attempts(run, qid, kind)}
        if name not in allowed or not citation.get('passage', '').strip():
            raise ValueError('Citation must name a retained attempt and nonempty passage')
        data = payload(allowed[name])
        if name.startswith('search-'):
            texts = [row.get('snippet', '') for row in data.get('results', []) if row['url'] == citation['url']]
        else:
            request = read(allowed[name] / 'request.json')
            texts = [data.get('content', '')] if request['argv'][2] == citation['url'] else []
        if not any(citation['passage'] in text for text in texts):
            raise ValueError('Supporting passage not found verbatim in cited evidence')
    return judgment


def report(run):
    manifest = read(run / 'manifest.json')
    if manifest['dataset_sha256'] != sha(run / 'queries.json'):
        raise ValueError('Frozen dataset changed')
    rows = []
    for question in canonical_questions(run / 'queries.json'):
        qid = question['id']
        times = {kind: 0.0 for kind in ('search', 'fetch')}
        incomplete = False
        for kind in times:
            for attempt in attempts(run, qid, kind):
                if not (attempt / 'receipt.json').exists():
                    incomplete = True
                else:
                    times[kind] += read(attempt / 'receipt.json')['seconds']
        path = run / qid / 'assessment.json'
        judgment = validate_assessment(run, qid, read(path)) if path.exists() else {'verdict': 'fail', 'rationale': 'Missing assessment'}
        rows.append({'id': qid, **judgment, 'search_seconds': times['search'], 'fetch_seconds': times['fetch'],
                     'total_tool_seconds': None if incomplete else sum(times.values()), 'incomplete_attempt': incomplete})
    passed = sum(row['verdict'] == 'pass' and not row['incomplete_attempt'] for row in rows)
    return {'gate': 'PASS (10/10)' if passed == 10 else f'NOT PASSED ({passed}/10)', 'manifest': manifest, 'questions': rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run', type=lambda value: Path(value).resolve())
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('init').add_argument('--assessor', required=True)
    search = sub.add_parser('search')
    search.add_argument('id', nargs='?')
    search.add_argument('--query')
    search.add_argument('--reason')
    fetch = sub.add_parser('fetch')
    fetch.add_argument('id')
    fetch.add_argument('url')
    fetch.add_argument('--reason', required=True)
    assess = sub.add_parser('assess')
    assess.add_argument('id')
    assess.add_argument('judgment', type=Path)
    sub.add_parser('report')
    args = parser.parse_args()
    if args.command == 'init':
        initialize(args.run, args.assessor)
    elif args.command == 'search':
        if args.id is None and (args.query or args.reason):
            raise ValueError('Recovery requires a question ID')
        ids = [args.id] if args.id else [row['id'] for row in read(args.run / 'queries.json')]
        for qid in ids:
            retrieve(args.run, qid, 'search', args.query, args.reason)
    elif args.command == 'fetch':
        retrieve(args.run, args.id, 'fetch', args.url, args.reason)
    elif args.command == 'assess':
        check(args.run)
        save(args.run / args.id / 'assessment.json', validate_assessment(args.run, args.id, read(args.judgment)))
    else:
        print(json.dumps(report(args.run), indent=2, ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError) as error:
        sys.exit(str(error))
