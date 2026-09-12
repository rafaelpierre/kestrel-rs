#!/usr/bin/env python3
"""Reduce a local Bing response to organic markup; remove tracking and active content.

The result is a structural excerpt, not a byte-identical response. Keep the raw
SHA-256 and capture provenance alongside it. Search terms and page text remain.
"""
import argparse
import base64
import hashlib
from html import escape
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import parse_qs, urlsplit, urlunsplit


def destination(value):
    parsed = urlsplit(value)
    if parsed.hostname in ('www.bing.com', 'bing.com') and parsed.path in ('/ck/a', '/cr'):
        params = parse_qs(parsed.query)
        if params.get('rurl'):
            value = params['rurl'][0]
        elif params.get('u'):
            encoded = params['u'][0]
            if encoded.startswith('a1'):
                encoded = encoded[2:]
            try:
                value = base64.urlsafe_b64decode(encoded + '=' * (-len(encoded) % 4)).decode()
            except (ValueError, UnicodeError):
                return ''
        else:
            return ''
    parsed = urlsplit(value)
    if parsed.scheme not in ('http', 'https') or not parsed.hostname:
        return ''
    # All query parameters removed deliberately: these are public text fixtures.
    return urlunsplit((parsed.scheme, parsed.hostname, parsed.path, '', ''))


class Excerpt(HTMLParser):
    allowed = {'li', 'h2', 'a', 'div', 'p', 'cite', 'strong', 'b', 'span'}

    def __init__(self, limit):
        super().__init__(convert_charrefs=True)
        self.limit, self.count, self.depth, self.skip = limit, 0, 0, 0
        self.output, self.title = [], []
        self.in_title = False

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if tag == 'title':
            self.in_title = True
        if not self.depth:
            if tag != 'li' or 'b_algo' not in values.get('class', '').split() or self.count >= self.limit:
                return
            self.count += 1
            self.depth = 1
        elif tag == 'li':
            self.depth += 1
        if tag in ('script', 'style', 'noscript'):
            self.skip += 1
        if self.skip or tag not in self.allowed:
            return
        kept = []
        classes = [c for c in values.get('class', '').split() if c in ('b_algo','b_caption','b_attribution')]
        if classes:
            kept.append(('class', ' '.join(classes)))
        if tag == 'a':
            kept.append(('href', destination(values.get('href', ''))))
        self.output.append('<' + tag + ''.join(f' {k}="{escape(v, quote=True)}"' for k,v in kept) + '>')

    def handle_endtag(self, tag):
        if tag == 'title':
            self.in_title = False
        if not self.depth:
            return
        if tag in ('script','style','noscript') and self.skip:
            self.skip -= 1
            return
        if not self.skip and tag in self.allowed:
            self.output.append(f'</{tag}>')
        if tag == 'li':
            self.depth -= 1

    def handle_data(self, data):
        if self.in_title:
            self.title.append(data)
        if self.depth and not self.skip:
            self.output.append(escape(data))


def sanitize(raw, limit=2):
    parser = Excerpt(limit)
    parser.feed(raw)
    return '<!doctype html>\n<!-- Sanitized live structural excerpt. -->\n<title>' + escape(''.join(parser.title)) + '</title>\n<ol>' + ''.join(parser.output) + '</ol>\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    raw = args.input.read_bytes()
    with args.output.open('x') as f:
        f.write(sanitize(raw.decode()))
    print('Original response SHA-256:', hashlib.sha256(raw).hexdigest())


if __name__ == '__main__':
    main()
