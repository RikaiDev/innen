# Provenance-guided source retrieval

After wiki edits, explicitly synchronize their graph projection:

```sh
innen wiki sync
innen trace --q 'release develop main' --role user
```

`trace` retrieves clues with BM25, follows bounded typed provenance paths, and
searches only the reached sources. Reverse `DELIVERED` links lead from an
artifact to its originating conversation. Membership paths are weak retrieval
associations, never evidence that one project's conversation belongs to another.
An optional `--seed <exact-node-id>` pins the starting node without changing
the source text query. It does not infer an ID from a filesystem directory.

Results carry the seed, source node, every graph hop and traversal direction,
source role and position, quoted text, version/hash, and structured
`expand_argv`. Execute that argv to recover the full selected source record;
expansion checks current source bytes and rejects a changed record. Role labels
describe source metadata, not proof that a sentence is an authorized decision.

Selected evidence files may themselves contain explicit session descriptors.
Recognized JSON fields (`session_id`, `conversation_id`, `origin_session_id`)
and labeled native-session lines are followed within the same source/depth/byte
budgets. The result records a `METADATA_SESSION_REF` hop with the parent source
hash and field pointer. This is a witnessed retrieval hint, not an asserted
causal relationship or authorization. Arbitrary UUIDs in ordinary prose are not
treated as conversation references.

The native scanner locates a selected session once and reads its JSONL once.
Private content-addressed caches live under `.innen/trace/`; no daemon, embedding
service or model inference is required. Warm cache checks use file metadata;
content verification remains part of exact expansion. `--refresh` rereads sources.
`--max-sources`, `--max-bytes`, `--max-records`, `--max-nodes`, and `--max-depth`
bound work; limitations and unavailable sources are explicit. No match in the
covered sources is not proof that no such conversation exists.

Logical source bytes, cache hits, attempted sources and scanned records are
reported separately. Locator/stat/read-ahead I/O is not included in the logical
byte counter. A failed scan with unknown consumption makes that counter null
and conservatively charges the remaining allowance; a missing file before any
scan consumes no logical source bytes. Expired graph edges are rechecked at
query time even when the journal cache has not changed.

Wiki graph synchronization is explicit. The graph reflects the most recent
successful sync, not unsynchronized edits to Markdown. External URLs are kept
as references and never fetched automatically. Unsupported source formats,
unresolved short session identities and missing source files remain diagnostics.

Trace tokenization streams lowercase Unicode alphanumeric runs and CJK
unigrams/bigrams, with the same representation for queries and indexed text.
CJK unigrams receive weight 0.2 to reduce isolated-character noise. It loads no
segmentation dictionary; the legacy query/index tokenizer remains unchanged.
Tokenization changes invalidate the versioned trace cache.

BM25 uses k1=1.2 and b=0.75. Source and local passage ranks are combined with
RRF (k=60). These are retrieval heuristics; neither ranking nor a short output
establishes semantic sufficiency or a subscription cost improvement. Cold index
cost, warm reuse, exact expansion, and reasonable Python baselines must be
measured separately on a fixed corpus before making efficiency claims.
