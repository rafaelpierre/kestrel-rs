# Lexical hybrid evidence pilot — issue #78

Keep the current lexical hybrid policy as an experimental option. This pilot does
not justify promoting every fetched page, tuning weights, or changing defaults.
Fetching produced useful support, but did not improve mean direct relevance in
these fixed pools. Selective reading and passage extraction (#83/#84) remain
better-scoped next experiments than a blanket preference for non-null bodies.

## Reproduction and controls

Base: `ec8024d` (7.0.0, including #190's two-second fetch default). Build with:

```sh
cargo build --release --locked --bin kestrel --example hybrid_evidence
python3 benchmarks/hybrid_evidence.py /tmp/issue78-study
python3 benchmarks/hybrid_ablation.py /tmp/issue78-study /tmp/issue78-ablation
python3 benchmarks/report_hybrid_evidence.py /tmp/issue78-study \
  benchmarks/hybrid-evidence-20260914/judgments.json
KESTREL_HYBRID_REPLAY="$PWD/target/release/examples/hybrid_evidence" \
  python3 -m unittest discover -s benchmarks -p test_hybrid_evidence.py
```

Use judgments for the exact captured content hashes; this committed judgment set
is for the retained September 14 run, not automatically for a new live run.
Unknown judgments remain null. Report schema version 2 places process/fetch
seconds, page attempts, response bytes and extraction counts in `arms`, once per
`(query_id, round, mode)`. Policy `rows` retain quality scores and URLs and join to
`arms` through those three keys. Consumers of the original unversioned report
must aggregate costs from `arms` instead of policy rows. These costs cover the
matched metadata/fetch arms only; the separate pre-ranking experiment remains
excluded. Summing fetched `arms` reproduces the 240-attempt study total. Full artifacts are local at
`/tmp/kestrel-78-study-v3/` and `/tmp/kestrel-78-ablation/`. Failed pilot attempts
`/tmp/kestrel-78-study/` and `-v2/` are retained: the former omitted the artifact
run ID, the latter used the wrong filename glob. Neither contributes measurements.

One discovery per query uses all default engines, native lexical passthrough,
`--no-fetch --no-rank --top-k 1000 --min-results 20 --search-budget 5 --output json`.
The full pre-ranking artifact pool is frozen; output JSON alone is not used to
infer omitted candidates. The four exact music queries come from
`quality-queries-v1.json`. Non-music q03/q04/q07/q10 use the canonical FTS dataset;
they were fixed before measurement and no weights were fitted to either set.
These are held-out *from music analysis*, not unseen agent-evaluation questions.

The example selects the first 15 candidates. Metadata and live-fetch processes
alternate order across two rounds, with 250 ms idle gaps. They rank identical
selected metadata. Each process replays provider/snippet/body/hybrid and retains
**every** output rank; analysis uses top five. Fetched policies within one process
see exactly the same bodies. Fetch has a two-second total budget, ten-second
request timeout, 2,000-character limit, 1 MB byte cap, fetch/parse concurrency ten,
no page cache, and the same PDF skip rule as the CLI. Provider recovery and remote
telemetry are disabled. All URLs, bodies, outcomes, byte counts, stdout/stderr,
argv, timestamps and exit codes remain in exclusive-created attempt directories.

The example is a benchmark interface, not a new CLI or library contract. It runs
four policies in each arm, so its process times include all four rankings. It
excludes discovery and does not imitate the full CLI's initialization path.
Do not call this a matched total-search benchmark. Source hashes and both binary
hashes are in the study manifest; the gate below records the production binary
independently. The report/ablation tools do not change production behavior or the
generated skill.

## Frozen ranking judgments

Relevance is 0 for wrong entities/generic pages, 1 for background, and 2 for direct
relevance. Evidence is 1 only when the retained body provides a substantive
supporting fact. It does not certify a complete answer; the ten-question gate has
a stricter, separate rubric. Successful extraction is counted independently.
The unblinded Codex GPT-6 assessor's URL/content-hash judgments are in
[judgments.json](judgments.json). The 120 prefix candidates were judged in both
rounds; two bodies changed without changing their supporting facts. The separate
pre-ranking arm was also judged against its own retained body hashes. No unknown
judgments were treated as zero. Precision divides direct results by five;
evidence coverage divides useful body slots by five, including missing slots.

Each cell below is **precision / evidence coverage** for round one. Round two had
the same aggregate policy scores. Snippet uses the fetched pool's attached bodies
for the evidence measure while ignoring them for scoring. Metadata-only processes
have zero *body* evidence, not necessarily zero answerable snippets.

| Query | Pool | Provider | Snippet | Body | Hybrid |
| --- | ---: | --- | --- | --- | --- |
| Broken Social Scene / Anthems | 25 | .4 / .4 | 1 / 1 | .8 / .8 | .8 / .8 |
| Metric / Scott Pilgrim | 28 | .8 / .6 | .8 / .4 | 1 / .8 | 1 / .4 |
| Sonic Youth / tunings | 20 | .2 / .2 | .4 / .2 | .4 / .2 | .4 / .2 |
| Kurt Vile / Courtney Barnett | 20 | .2 / .2 | .8 / .8 | .8 / .8 | .8 / .8 |
| q03 TaskGroup | 25 | .2 / .2 | .6 / .2 | .6 / .4 | .6 / .4 |
| q04 EXPLAIN | 20 | .4 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| q07 Rust E0382 | 25 | .2 / 0 | .6 / .4 | .8 / .4 | .6 / .4 |
| q10 museum hours | 28 | .6 / .4 | .8 / .2 | .4 / .2 | .8 / .2 |

Pool counts are live observations, not retrieval guarantees. See the retained
pool files for exact membership and provider/query provenance.

## Why bodies do and do not help

* **Broken Social Scene:** hybrid promotes the Genius song list to first, with
  a retained association between the band and the requested song. But NPR's
  *Hug of Thunder* review enters fifth and pushes the direct Genius song page
  out. Its repeated band name and generic “anthems” match the lexical query;
  it does not identify the requested song. This is a relevance regression
  relative to snippet (five direct/evidenced pages versus four).
* **Metric:** hybrid retains two bodyless YouTube results in positions two and
  four. Their metadata directly identifies “Black Sheep” and the film, whereas
  fetched film-review and band-biography alternatives do not establish that
  contribution in the retained prefix. Retaining them is correct for relevance;
  it supplies no retrieved page evidence. A SoundCloud result is non-null but
  contains HTML/error/recording metadata; it does not establish the film
  contribution. Fetched ABKCO vinyl tracklisting and Genius text do provide it.
* **Sonic Youth:** the Stringjoy tuning primer ranks first but image markup
  consumes much of its prefix; no specific tuning survives. The EP article
  supplies an E-standard-tuning fact. Other high-ranked album/biography pages
  establish association with the band, not concrete tunings. Body presence does
  not solve truncation or source selection.
* **Kurt Vile:** hybrid puts *Lotta Sea Lice* first. The Barnett biography,
  discography and awards page all retain the joint album name. The numerous
  fetched Kurt Geiger/Cobain pages are wrong entities despite successful fetches.
* **TaskGroup:** the bodyless GeeksforGeeks TaskGroup result remains fifth;
  its relevant metadata outranks generic fetched Python home/download/tutorial
  pages. PEP 654 adds actual grouped-exception evidence. Several other fetched
  prefixes are introductory, so this pool still cannot answer the full gate.
* **PostgreSQL:** hybrid's top four are bodyless mailing-list messages. Matching
  terms in doubled titles/snippets outrank short generic reference metadata.
  Neither they nor the fetched commitfest page supplies the required answer.
  Missing bodies here are chiefly an extraction/selection limitation; giving
  the commitfest page an automatic fetch-success boost would not fix it.
* **Rust:** the official E0382 page supplies move/borrow/clone evidence. Game
  pages are unrelated. Hybrid retains a GitHub book page with a short shell
  prefix above the official reference; source quality is not a lexical score.
* **Museum:** hybrid's top five contain only one usable opening-hours body.
  The official visitor page remains outside the top five despite a complete
  hours statement. Two directory pages retain navigation, and Time Out's
  2,000-character prefix stops before its hours. A separate fetched article
  claims a Wednesday–Sunday schedule that conflicts with the official daily
  schedule; it receives relevance credit but no trustworthy evidence credit.

The mechanism follows the production ranker: positive BM25 over doubled title,
snippet, and available body, with corpus-dependent IDF and length normalization.
It has no semantic authority/date/evidence score. Adding a body changes both its
own score and corpus statistics. Hybrid keeps public `bm25_score` unchanged;
body policy alone assigns content BM25. No new score API or scoring implementation
was introduced for this experiment.

## Live cost and controlled interventions

Across 16 matched pairs, metadata hybrid mean precision was **.625**, and fetched
hybrid was **.625**. Fetched body-evidence coverage was **.400**, versus zero
retained bodies in metadata mode. Metadata snippets can already answer some
questions, so .400 is not an answer-accuracy improvement estimate.

Metadata process median was **.0224 s**; fetched process median was **1.3751 s**.
The fetched arms recorded **240 page attempts**, **67,754,777 decoded response
bytes**, and **198 successful nonempty extractions**. These are incremental
page costs; shared discovery requests/bytes are outside this comparison.
Per-process receipts and fetch-phase times are retained. Nearest-rank percentile
convention is used; only a median is reported, since 16 within-eight-query samples
cannot establish reliable population tails. Upstream caching, live content,
network state and concurrent machine load are uncontrolled.

For round-one bodies, replay preserves candidate metadata, source text and corpus
membership while setting all bodies missing, truncating existing bodies to 200
characters, or replacing existing bodies with the same boilerplate. The last
intervention isolates availability/length effects, not realistic page quality.
The table counts shared top-five URLs with the original hybrid order (order
changes within the set are retained in raw output, not hidden as equivalence).

| Query | Missing | 200 chars | Boilerplate | Pre-rank selected overlap / 15 |
| --- | ---: | ---: | ---: | ---: |
| music-bss | 4 | 4 | 4 | 10 |
| music-metric | 4 | 5 | 5 | 10 |
| music-sonic | 5 | 5 | 5 | 11 |
| music-duet | 5 | 5 | 5 | 12 |
| q03 | 4 | 5 | 4 | 8 |
| q04 | 5 | 5 | 4 | 10 |
| q07 | 5 | 5 | 4 | 9 |
| q10 | 5 | 5 | 5 | 10 |

Pre-ranking is a **separate** live selection arm. It changes 3–7 selected URLs,
so attributing its results to final ranking would be invalid. Its newly selected
pages are not included in the fixed-prefix precision estimates above. Its separate
hybrid precision/evidence scores were: music-bss 1/.8, music-metric .8/.4,
music-sonic .4/.2, music-duet .8/.8, q03 .4/.2, q04 0/0, q07 .6/.4, q10 .8/.4.
The mixed outcomes do not justify changing the pre-ranking default. This arm has
one live repeat and different selected bodies, so it is not a final-ranker ablation.

Synthetic one/two/three-candidate controls, single- and two-query groups, replace
only one fixed candidate's body with missing/useful/boilerplate/truncated text.
They exercise rank stability and fair interleaving without live variation; their
full outputs are retained under `synthetic-*`. They are mechanism controls, not
natural-query quality estimates. Missing-body hybrid and snippet retain all
candidates, unlike content filtering; equal-score order remains deterministic.

### Body-intervention relevance and evidence

The 200-character prefixes were rejudged for retained supporting facts; original
evidence labels were not blindly reused. Missing and boilerplate bodies receive
zero evidence. Relevance remains a judgment of the underlying page against the
fixed intent. Each cell is precision/evidence for hybrid.

| Query | Original | Missing | 200 chars | Boilerplate |
| --- | --- | --- | --- | --- |
| music-bss | 0.8/0.8 | 1.0/0.0 | 0.8/0.2 | 1.0/0.0 |
| music-metric | 1.0/0.4 | 0.8/0.0 | 1.0/0.2 | 1.0/0.0 |
| music-sonic | 0.4/0.2 | 0.4/0.0 | 0.4/0.0 | 0.4/0.0 |
| music-duet | 0.8/0.8 | 0.8/0.0 | 0.8/0.2 | 0.8/0.0 |
| q03 | 0.6/0.4 | 0.6/0.0 | 0.6/0.2 | 0.8/0.0 |
| q04 | 0.0/0.0 | 0.0/0.0 | 0.0/0.0 | 0.0/0.0 |
| q07 | 0.6/0.4 | 0.6/0.0 | 0.6/0.2 | 0.4/0.0 |
| q10 | 0.8/0.2 | 0.8/0.0 | 0.8/0.0 | 0.8/0.0 |

## Decision and limitations

Keep hybrid experimental and preserve bodyless candidates. The current evidence
supports selective retrieval experiments (#84), focused reading (#83), and
provider-fusion evaluation (#101), with extraction investigation #157 linked for
q04. No isolated implementation defect was established, so this PR changes no
production weights/defaults and files no speculative ranker correction.

This is a small unblinded pilot with two repeats, partial-support judgments and
no independent assessor. Larger held-out answer-level validation remains the
separately scoped work in #84; this pilot is an evidence-backed keep recommendation
for the shipped lexical policy. **#78 stays open while its required gate fails**. The separate gate is **NOT PASSED (9/10)**; see
[evidence-gate.md](evidence-gate.md). No merge/readiness claim follows from the
ranking observations.
