"""Explicit current-CLI policies for the quality/latency runner (schema version 2)."""
import json

ENGINES = ['duckduckgo', 'bing', 'yahoo', 'dogpile', 'ecosia', 'swisscows', 'yep', 'qwant', 'mojeek']
STAGES = ['retrieval', 'providers', 'ranking', 'candidates', 'pre-rank', 'fetching',
          'concurrency', 'fetch-budget', 'search-budget', 'swisscows']


def conditions(stage, engine=None, *, top_k=5, minimum=15, pool=15,
               search_budget=None, fetch_budget=None, syntax='passthrough', cache='off'):
    if stage not in STAGES:
        raise ValueError(f'unknown stage: {stage}')
    if min(top_k, minimum, pool) < 1:
        raise ValueError('top-k, minimum and candidate pool must be positive')
    if stage == 'search-budget' and search_budget is not None:
        raise ValueError('common search budget cannot override its ablation stage')
    if stage == 'fetch-budget' and fetch_budget is not None:
        raise ValueError('common fetch budget cannot override its ablation stage')
    base = dict(engines=[engine] if engine else ENGINES, minimum=minimum, top_k=top_k,
                candidate_cap=pool, query_syntax=syntax, ranking='hybrid', fetch=True,
                pre_rank=False, min_fetch_score=None, search_budget_seconds=search_budget if search_budget is not None else 5.0,
                search_budget_mode='explicit' if search_budget is not None else 'cli-default',
                fetch_budget_seconds=fetch_budget if fetch_budget is not None else 5.0,
                cache=cache, cache_ttl_seconds=3600 if cache != 'off' else None,
                cache_max_entries=1000 if cache != 'off' else None, search_concurrency=10, fetch_concurrency=10,
                parse_concurrency=10, page_timeout_seconds=10.0, content_limit=2000,
                max_response_bytes=1000000, region='', time_filter='any')
    changes = []
    if stage == 'retrieval':
        changes = [(f'minimum-{n}', dict(minimum=n, fetch=False, ranking='none')) for n in [1, 5, 15]]
    elif stage in ['providers', 'swisscows']:
        changes = [(e, dict(engines=[e], fetch=False, ranking='none'))
                   for e in ([engine or 'swisscows'] if stage == 'swisscows' else ENGINES)]
    elif stage == 'search-budget':
        changes = [(f'search-{n}s', dict(fetch=False, ranking='none', search_budget_seconds=float(n),
                                       search_budget_mode='explicit')) for n in [1, 2, 3]]
        changes += [('search-default-5s', dict(fetch=False, ranking='none')),
                    ('search-no-total-deadline', dict(fetch=False, ranking='none',
                     search_budget_seconds=None, search_budget_mode='unlimited'))]
    elif stage == 'fetching':
        changes = [('metadata-snippet', dict(fetch=False, ranking='snippet')),
                   ('fetched-snippet', dict(ranking='snippet'))]
    elif stage == 'ranking':
        changes = [(p, dict(ranking=p)) for p in ['provider', 'snippet', 'body', 'hybrid', 'rrf']]
    elif stage == 'candidates':
        if minimum < 15 or pool < 15:
            raise ValueError('candidate sweeps require minimum and candidate pool >= 15')
        changes = [(f'candidates-{n}', dict(candidate_cap=n)) for n in [5, 10, 15]]
    elif stage == 'pre-rank':
        changes = [('pre-rank-off', {}), ('pre-rank-on', dict(pre_rank=True))]
    elif stage == 'concurrency':
        changes = [(f'concurrency-{n}', dict(fetch_concurrency=n)) for n in [5, 10]]
    elif stage == 'fetch-budget':
        changes = [(f'fetch-{n}s', dict(fetch_budget_seconds=float(n))) for n in [1, 2, 5, 20]]
    result = []
    for name, change in changes:
        policy = base | change
        if not policy['fetch']:
            if cache != 'off' or fetch_budget is not None:
                raise ValueError('metadata-only conditions cannot use page cache or a common fetch budget')
            policy.update(candidate_cap=None, pre_rank=False, fetch_budget_seconds=None,
                          fetch_concurrency=None, parse_concurrency=None, page_timeout_seconds=None,
                          content_limit=None, max_response_bytes=None, cache='not-applicable')
        result.append(dict(name=name, effective=policy, flags=flags_for(policy)))
    check_duplicates(result)
    return result


def check_duplicates(items):
    seen = {}
    for item in items:
        # Omission and an explicit five-second deadline have the same effect.
        policy = {k: v for k, v in item['effective'].items() if k != 'search_budget_mode'}
        key = json.dumps(policy, sort_keys=True)
        if key in seen:
            raise ValueError(f'duplicate effective conditions: {seen[key]} and {item["name"]}')
        seen[key] = item['name']


def flags_for(p):
    flags = ['--mode', 'fanout', '--min-results', str(p['minimum']),
             '--top-k', str(p['top_k']), '--search-concurrency', str(p['search_concurrency']),
             '--region', p['region'], '--time-filter', p['time_filter']]
    if p['query_syntax'] != 'passthrough':
        flags += ['--query-syntax', p['query_syntax']]  # Historical executable replay only.
    for engine in p['engines']:
        flags += ['--engine', engine]
    if p['search_budget_mode'] == 'unlimited':
        flags += ['--no-search-budget']
    elif p['search_budget_mode'] == 'explicit':
        flags += ['--search-budget', str(p['search_budget_seconds'])]
    if p['ranking'] == 'none':
        flags += ['--no-rank']
    else:
        flags += ['--ranking-policy', p['ranking']]
    if not p['fetch']:
        return flags + ['--no-fetch']
    for flag, key in [('--fetch-candidates', 'candidate_cap'), ('--fetch-budget', 'fetch_budget_seconds'),
                      ('--concurrency', 'fetch_concurrency'), ('--parse-concurrency', 'parse_concurrency'),
                      ('--timeout', 'page_timeout_seconds'), ('--content-limit', 'content_limit'),
                      ('--max-response-bytes', 'max_response_bytes')]:
        flags += [flag, str(p[key])]
    return flags + (['--pre-rank'] if p['pre_rank'] else [])
