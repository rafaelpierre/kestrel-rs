#!/usr/bin/env python3
"""Build a release-evidence coding pilot from already captured primary-source releases."""
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).parent / 'coding-pilot-20260911'
# Repository-disjoint splits prevent development cases sharing releases with held-out cases.
COUNTS = [('tokio-rs/tokio',2,'development'), ('serde-rs/serde',1,'development'),
          ('clap-rs/clap',2,'development'), ('hyperium/hyper',2,'development'),
          ('seanmonstar/reqwest',3,'development'), ('astral-sh/ruff',6,'held_out'),
          ('astral-sh/uv',6,'held_out'), ('pydantic/pydantic',4,'held_out'),
          ('pytest-dev/pytest',4,'held_out')]


def main():
    questions, answers = [], []
    for repo, count, split in COUNTS:
        path = ROOT / 'sources' / (repo.replace('/','--') + '.json')
        releases = json.loads(path.read_text())
        selected = []
        for release in releases:
            section = ''
            for line in release['body'].splitlines():
                if line.startswith('#'):
                    section = line.lstrip('# ').lower()
                if not re.match(r'\s*[-*] ', line) or line.rstrip().endswith('**:'):
                    continue
                if 'document' in section:
                    continue
                # Prefer a PR/issue identifier so the question does not supply the behavior answer.
                links = re.findall(r'https://github.com/[\w.-]+/[\w.-]+/(?:pull|issues)/\d+', line)
                if not links:
                    ids = re.findall(r'#(\d+)', line)
                    links = [f'https://github.com/{repo}/pull/{i}' for i in ids[:1]]
                symbols = re.findall(r'`([^`]+)`', line)
                subject = f'change linked to {links[0]}' if links else (f'change involving `{symbols[0]}`' if symbols else line.lstrip('-* '))
                selected.append((release, line.strip(), subject, links))
                if len(selected) == count:
                    break
            if len(selected) == count:
                break
        if len(selected) != count:
            raise ValueError(f'{repo}: need {count} reviewed change lines, found {len(selected)}')
        for release, passage, subject, links in selected:
            id_ = f'coding-{len(questions)+1:02d}'
            published = release['published_at'][:10]
            if not '2026-06-13' <= published <= '2026-09-11':
                raise ValueError('source outside freshness window')
            questions.append(dict(id=id_, split=split, repository=repo, as_of='2026-09-11',
                question=f'In {repo}, identify the release published on {published} that lists the {subject}. What behavior, API, or dependency changed? Give the release tag and cite the release notes.'))
            answers.append(dict(id=id_, expected_release=release['tag_name'], repository=repo,
                required_facts=[f'Release tag: {release["tag_name"]}', passage],
                gold_urls=[release['html_url'], *links], supporting_passage=passage,
                source_published_at=release['published_at'], snapshot=str(path.relative_to(ROOT)),
                snapshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                assessment='Release membership and change summary verified against captured publisher release notes. Linked PR execution and discoverability remain to be checked.'))
    (ROOT/'questions.json').write_text(json.dumps(questions,indent=2)+'\n')
    (ROOT/'answer-key.json').write_text(json.dumps(answers,indent=2)+'\n')
    print(f'{len(questions)} questions: {sum(q["split"]=="development" for q in questions)} development, {sum(q["split"]=="held_out" for q in questions)} held out')


if __name__ == '__main__':
    main()
