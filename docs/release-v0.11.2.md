# innen v0.11.2

The structure debt is cleared. No behaviour changed in this release, which makes
the interesting part what the work turned up on the way.

## Changed

### Nine more files cut along their seams

| before | after | largest |
| --- | --- | --- |
| `task_entry.rs` 1046 | types / query_terms / node_identity / admission / traversal / entry / render / tests | 350 |
| `evidence_closure.rs` 783 | contract / snapshot / closure / evaluation / tests | 165 |
| `task_proposal.rs` 758 | proposal / id_context / review / evaluate / tests | 142 |
| `trace/source.rs` 524 | schema / read_file / read_native / store / mod | 179 |
| `session_retention.rs` 506 | types / support / inventory / attest / purge | 134 |
| `conversation/unfinished.rs` 481 | model / since / discover / evidence | 217 |
| `conversation/grammar.rs` 474 | tokens / compress / encode / decode / tests | 253 |
| `conversation/projection.rs` 467 | dialogue / index / context / tests | 202 |
| `harvest.rs` 468 | tap / report / scan / tests | 216 |

`task_entry` could not be done by moving items: `task_entry` alone is 468 lines,
so two self-contained blocks were extracted verbatim into
`literal_candidate_scores` and `traverse_candidates`. Every other file here is a
file-level move with visibility widened only where the move forced a sibling to
read something.

### The debt list has no unpaid entries

Eight entries remain and all eight are exemptions, each re-checked rather than
carried over: production code under the 400-line budget (`artifact` 384,
`index` 341, `query` 398, `credentials` 394, `context_selection` 297), or a pure
test module holding no production code at all (`graph/tests.rs`,
`doctor/tests.rs`, `session_retention/tests.rs`).

The gate continues to refuse an entry that names neither a cut nor an exemption,
one that names both, and the `TODO` placeholder.

## Fixed

### An excerpt window could be silently shifted

`trace`'s `excerpt` searched `text.to_lowercase()` for the match and then indexed
`text` with the result. Where lowercasing changes the byte length, the offset
belongs to a different string. `İstanbul` is nine bytes and lowercases to ten,
so a search for `stanbul` returns offset 2 — which in the original is `s`, not
the `t` the search started from.

No panic: `start` is walked back to a char boundary, so the failure is a quietly
wrong excerpt rather than a crash. This is pre-existing and unrelated to the
split; it was found while verifying the `trace/source` cut.

## Known issues, unchanged by this release

Found by the splits, deliberately not fixed here because a refactor is the wrong
place to change behaviour:

- `session_retention/policy.rs` repeats the 21-line "bundle already removed"
  early return verbatim. The first copy always returns, so the second is dead
  code — including its comment. It is a copy-paste artefact of the v0.10.2
  `already_cleaned` fix.
- `task_entry::CandidateItem` is defined and re-exported but never constructed
  anywhere in the workspace.
- `trace/source`: `materialize`'s `edge.assert` / `edge.retract` arms call
  `typ.parse().unwrap()`, sound only because `EdgeType::Err = Infallible`.

## Measured

| | v0.11.1 | v0.11.2 |
| --- | --- | --- |
| files over the 400-line budget | 17 | 8 |
| of those, real code debt | 9 | 0 |
| largest non-test file | 367 | 350 |
| source files | 139 | 183 |

403 tests pass, 0 failed. `cargo clippy --workspace --all-targets
--all-features --locked -- -D warnings` clean. `cargo fmt --all --check` clean.