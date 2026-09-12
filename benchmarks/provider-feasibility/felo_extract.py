#!/usr/bin/env python3
"""Extract sources from a completed Felo page, never initiate a search.

Usage: python3 felo_extract.py captured-thread.html
Input is an actual HTML response or the sanitized fixture projection.
"""
import json
from html.parser import HTMLParser
from pathlib import Path
import sys
from urllib.parse import urlsplit


class NextData(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=False)
        self.inside = False
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag == 'script' and dict(attrs).get('id') == '__NEXT_DATA__':
            self.inside = True

    def handle_endtag(self, tag):
        if tag == 'script':
            self.inside = False

    def handle_data(self, data):
        if self.inside:
            self.parts.append(data)


def extract(html):
    parser = NextData()
    parser.feed(html)
    if not parser.parts:
        raise ValueError('unrecognized HTML: missing __NEXT_DATA__')
    data = json.loads(''.join(parser.parts))
    threads = data.get('props', {}).get('pageProps', {}).get('threads')
    if not isinstance(threads, list) or not threads:
        raise ValueError('entry page or missing thread data; not a valid empty search')
    output = []
    for thread in threads:
        if not isinstance(thread, dict) or thread.get('status') != 'completed':
            raise ValueError('incomplete or malformed thread')
        query, contexts = thread.get('query'), thread.get('recall_contexts')
        if not isinstance(query, str) or not isinstance(contexts, list):
            raise ValueError('missing query or source collection')
        # Even a completed thread with no sources may be an answer-only task.
        # No live valid-empty search contract was observed.
        if not contexts:
            raise ValueError('no sources: valid-empty semantics unverified')
        rows = []
        for rank, source in enumerate(contexts, 1):
            if not isinstance(source, dict):
                raise ValueError('malformed source')
            link = source.get('link')
            if not isinstance(link, str):
                raise ValueError('missing source URL')
            parsed = urlsplit(link)
            if parsed.scheme not in ('http', 'https') or not parsed.hostname or parsed.username:
                raise ValueError('invalid source URL')
            row = {'engine': 'felo', 'query': query, 'engine_rank': rank, 'url': link}
            for key in ('title', 'snippet'):
                value = source.get(key)
                if value is not None and not isinstance(value, str):
                    raise ValueError('malformed source field')
                row[key] = value  # Preserve missing fields; do not invent text.
            rows.append(row)
        output.append({'query': query, 'results': rows})
    return output


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit('usage: felo_extract.py captured-thread.html')
    try:
        print(json.dumps(extract(Path(sys.argv[1]).read_text()), ensure_ascii=False, indent=2))
    except (ValueError, OSError) as error:
        sys.exit(str(error))
