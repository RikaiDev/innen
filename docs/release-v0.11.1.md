# innen v0.11.1

A retrieval defect found by using the tool, a bookkeeping control that had been
saying nothing, and eight files cut along the seams they already had.

## Fixed

### A node could not be found by its own id

`innen query --q <node-id>` returned nothing for a node that was in the journal,
under its own id, with a label and a body. Its **body text** was findable; its id
was not.

v0.10.2 had already tried to fix this and fixed half of it. It exempted an exact
id match from the all-terms admission gate — but the id match still had to arrive
from the literal substring test, and that test skips its id branch whenever the
label looks path-like (`workspace:`, or any `/` in it). A node labelled
`session/repo` therefore never produced a candidate, so the gate it had been
exempted from was never consulted at all.

The id match is now a seed in its own right. The path-like guard still keeps an
unrelated path-like node out of an exact-id query, which the regression test
pins.

The earlier test passed while this was broken because its label contained no
slash. It asserted the outcome for one label shape, and that was read as the
contract. Worth naming: a narrow test is worse than no test, because it reports
confidence it has not earned.

## Changed

### The debt list now has to say how the debt gets paid

Every one of the 24 entries in `scripts/structure-baseline.json` carried
`split_by: "TODO"`, so the list recorded that debt existed and nothing about how
it would be settled. The gate accepted those entries — an unnamed entry was
indistinguishable from a decided one, so all 24 read as planned while planning
nothing.

An entry now states exactly one of:

- `split_by` — the seams the file will be cut along
- `exempt` — why it is not debt

never both, never neither, never `TODO`.

`--write-baseline` also used to overwrite every `split_by` with `"TODO"` on each
regeneration, so a recorded plan was destroyed by the act of keeping the list
current. It now re-measures `lines` and preserves `split_by` and `exempt` for
files already listed.

Six entries are `exempt` because their production code is already under budget
and the overage is inline tests, including `session_retention/tests.rs`, which is
pure test code with no responsibility to separate at all. Splitting tests to
satisfy a line number changes no design.

### Eight files split by single responsibility

| Before | After | Largest file |
| --- | --- | --- |
| `graph.rs` 1476 | `graph/{mod,adjacency,materialize,requests,tests}.rs` | 354 |
| `doctor.rs` 1072 | `doctor/{mod,paths,journal,quarantine,index,refs,tap,tests}.rs` | 551 (tests) |
| `middleware/history.rs` 929 | `history/{mod,model,ancestry,expand,prepare,tests}.rs` | 337 |
| `config.rs` 807 | `config/{mod,layers,merge,global,root,tests}.rs` | 215 |
| `conversation.rs` 642 | `conversation/{mod,record,bounded,read,format}.rs` | 250 |
| `conversation/resume.rs` 798 | `resume/{mod,antigravity,codex,dash,opencode,time}.rs` | 275 |
| `conversation/prune.rs` 848 | `prune/{mod,classify,scan,rules,pair,apply,page,tests}.rs` | 269 (tests) |
| `artifact/ledger.rs` 719 | `ledger/{mod,model,ops,state,fs}.rs` | 367 |

No behaviour changed. Every public path that resolved before resolves now; the
`conversation` and `graph` facades keep all 12 and 15 original re-exports
respectively, verified by set comparison against `HEAD` rather than by eye.

Two seams worth recording, because both were found by the split rather than
chosen up front:

- `conversation/resume.rs` lines 755-798 are `system_time_to_rfc3339` and
  `days_to_ymd`, not `deduplicate_candidates` as the seam map claimed.
  `system_time_to_rfc3339` is `pub(crate)` and consumed by
  `conversation/unfinished.rs` and `conversation/checkpoint.rs`, so it is not
  internal plumbing and could not simply move into a scan-helper module.
- `doctor`'s check order and exit-code mapping are observable through
  `innen doctor`, so the split could not reorder or rename checks.

## Known issues, unchanged by this release

Found while splitting, deliberately not fixed here because a refactor commit is
the wrong place to change behaviour:

- `graph/requests.rs`: the URI-legality fallback in
  `validate_relate_request_strict` has drifted structurally from the equivalent
  block in `validate_relate_request`. Behaviourally equivalent as written; two
  copies of one rule.
- `graph/materialize.rs`: the `edge.assert` / `edge.retract` arms call
  `typ.parse().unwrap()`. Sound only because `EdgeType::Err = Infallible`.

## Measured

| | Before | After |
| --- | --- | --- |
| `query --q <own id>` on a path-like label | 0 rows | 1 row, score 1.0, `literal_match` |
| baseline entries with no recorded plan | 23 | 0 |
| recorded plans surviving `--write-baseline` | no | yes |
| files over the 400-line budget | 23 | 17 |
| of those, real code debt | 17 | 9 |
| largest non-test file | 1476 | 367 |

403 tests pass, 0 failed. `cargo clippy --workspace --all-targets
--all-features --locked -- -D warnings` clean. `cargo fmt --all --check` clean.

## Debt still outstanding

This release clears 8 of the 17 oversized files. Nine still carry a recorded
`split_by` and are not yet split:

`conversation/grammar.rs`, `conversation/projection.rs`,
`conversation/unfinished.rs`, `evidence_closure.rs`, `harvest.rs`,
`session_retention.rs`, `task_entry.rs`, `task_proposal.rs`, `trace/source.rs`.

Each names the seams it will be cut along, so the next pass starts from a plan
rather than from a line count. The eight remaining entries are `exempt`: their
production code is already under budget and the overage is inline tests.