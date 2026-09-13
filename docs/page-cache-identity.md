# Persistent page identity (#19)

Page text uses a versioned SHA-256 key over the parsed request URL and extracted
character limit. The current version is `page-text-v2`. Unlike search-result
canonicalization, it preserves non-root trailing slashes, meaningful and tracking
query parameters, parameter order and duplicates, and encoded path distinctions
such as `/a%2Fb` versus `/a/b`. URL fragments are removed because they are not sent
in an HTTP request. The URL parser's standard normalization of host case and
default ports remains valid. Invalid URLs retain their exact input key; network
validation still determines whether they can be fetched.

Old unversioned keys are not read or migrated: those entries can contain text from
an aliased resource or an older extractor. They remain eligible for existing
capacity pruning. No user directory is deleted as a migration step. A cold miss
on the first request after upgrading is expected; subsequent compatible requests
can hit the new key. Future extraction changes must bump the identity version.

The public `PageCache` API, flags, JSON and TTL/capacity defaults are unchanged.
Byte-capped pages remain excluded; content limits remain exact-match. This change
does not modify search's heuristic result deduplication, infer redirect aliases,
or add provider-progress recovery, authentication-varying representations or
byte-prefix resumption. Cached-operation budget accounting remains owned by #18.

Unit tests cover query/path distinctions, permitted fragment/host/default-port
aliases, character limits and legacy invalidation. A local HTTP integration test
serves distinct slash and encoded-path resources, then repeats requests with a
new client. It asserts correct independent text and one request per resource over
the cold and warm runs. Generated-skill coverage checks the identity and migration
guidance against the CLI help.
