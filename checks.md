# checks.md — innen 0.11.1 / 0.11.2

Independent reviewer: none available (single-repo, no second agent with a fresh
context). A separate self-review round was run instead — see "Self-review" below.

## v0.11.1

| requirement | pass/fail | evidence | fix |
|---|---|---|---|
| Harvest the pending snapshots | pass | `innen hook pending` → `{"pending":[]}`; `innen doctor` exit 0 | — |
| A node is findable by its own id | pass | `query --q "harvest-gap:stop-hook-snapshot-volume"` → `retrieved \| rows: 1 \| score 1.0 \| literal_match`, on the released binary `/opt/homebrew/bin/innen` | — |
| Regression test covers the live defect | pass | `tests/task_entry_precision.rs::exact_node_id_is_retrievable_even_when_the_label_looks_path_like`; the first version of this test passed while the defect was live because its label had no `/` | — |
| Debt entries name a cut or an exemption | pass | 23 entries, 0 with `split_by: "TODO"`; gate refuses neither/both/TODO — verified by mutating the baseline and observing each refusal | — |
| `--write-baseline` preserves recorded plans | pass | set `graph.rs` `split_by`, regenerate, read back: preserved | — |
| Eight files split, no behaviour change | pass | `cargo test --workspace --all-features --locked` → 403 passed / 0 failed; clippy `-D warnings` clean; `fmt --check` clean | — |
| Public surfaces unchanged | pass | mechanical extraction from `git show HEAD:<path>` (plain `git`; `rtk git show` truncates 758 lines to 229 and silently drops names) compared against each new facade | — |
| Release published | pass | `https://github.com/RikaiDev/innen/releases/tag/v0.11.1`; CI security + test/macos + test/ubuntu all success; Homebrew 0.11.0 → 0.11.1 | — |

## v0.11.2

| requirement | pass/fail | evidence | fix |
|---|---|---|---|
| Nine remaining files split | pass | commit `576e237`; largest file 350 (`task_entry/entry.rs`) | — |
| No unpaid debt entries | pass | `scripts/check-structure.py --write-baseline` → 8 entries, `split_by` count 0; gate `ok (183 files…)` | — |
| All 8 remaining entries re-checked, not carried over | pass | production code measured: artifact 384, index 341, query 398, credentials 394, context_selection 297 — all under 400; the other three are pure test modules | — |
| `trace::excerpt` offset defect fixed or reported | reported | `docs/release-v0.11.2.md` §Fixed: offset computed on `text.to_lowercase()`, applied to `text`; `İstanbul` 9 bytes → 10, so offset 2 is `s` not `t`. **Not fixed** — see Open problems | — |
| Tests pass | pass | 403 passed / 0 failed, 30 `test result: ok` lines, exit 0, after `cargo fmt --all` converged | — |
| Clippy strict + fmt clean | pass | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → no output; `cargo fmt --all -- --check` → 0 `Diff in` | — |
| Release published | pass | `https://github.com/RikaiDev/innen/releases/tag/v0.11.2`; CI security + test/ubuntu + test/macos all success on `ae87bcf` | — |

## v0.11.3 — the three findings from v0.11.2, fixed

| requirement | pass/fail | evidence | fix |
|---|---|---|---|
| `trace::excerpt` returns a window containing the search term | pass | `trace/source/excerpt_tests.rs`; the regression test FAILS on the pre-fix code (`head "…zzzzzz…"`, term absent) and passes after | replaced the lowercased-copy search with a case-insensitive comparison over `text` itself, so no offset can drift |
| The dead 21-line early return is gone | pass | `session_retention/policy.rs` lines 18-38 and 39-59 compared byte-identical before deletion; the first always returns, so the second was unreachable | deleted |
| `task_entry::CandidateItem` removed | pass | whole-workspace grep found exactly two occurrences: the definition and the re-export; never constructed | deleted, re-export dropped |
| No panic path left in graph materialization | pass | `typ.parse().unwrap_or_else(\|never\| match never {})` × 3; `EdgeType::Err = Infallible` so behaviour is identical | — |
| Tests pass | pass | 407 passed / 0 failed (403 before + 4 new excerpt tests) | — |

## Self-review round

Read the actual diffs rather than the agents' reports, and found four things the
reports did not surface:

1. `conversation::brief` was missing from the new facade — caught only because
   three integration tests stopped compiling. My earlier "31 public names all
   present" check compared *module declarations* against a *hand-written list*.
2. `policy.rs` repeats a 21-line early return verbatim; the first copy always
   returns, so the second is dead code.
3. `task_entry::CandidateItem` is defined and re-exported but never constructed
   anywhere in the workspace.
4. The recorded `split_by` for `session_retention.rs` described two
   responsibilities that already lived in `policy.rs` and `sweep.rs` — the plan
   was written from a `grep` outline without reading the file.

A second round, on the v0.11.3 changes, found two more:

5. The first two versions of the `excerpt` regression test passed against the
   broken code. Confirmed the third version fails on the unfixed implementation
   with a standalone `rustc` probe before committing it.
6. Deleting `CandidateItem` left its `serde` imports unused; clippy caught it.

### Verification that was invalid, and what it cost

Two local `cargo clippy` runs returned no error output and were read as passing.
Both had been refused by the singleflight wrapper with exit 75 and a one-line
diagnostic, because the machine was below its VM storage floor. `grep` over that
one line is empty. A tag was moved onto the resulting commit; CI rejected it with
two real clippy errors — `trace::source::version` and
`clippy::type_complexity` in `task_entry/traversal.rs` — neither of which any
local run had reported.

Recording this because the failure is not exotic: an empty result from a command
that did not execute reads exactly like a passing result, and the exit code of a
pipeline is the exit code of its last stage.

## Open problems (not done)

All three code findings from v0.11.2 are fixed in v0.11.3. One remains, and it is
not innen's to close.

| problem | status | evidence |
|---|---|---|
| `fansee/the-mirror`: 455 uncommitted files, last commit 2026-10-06, idle 2 days | **open — not innen's data** | `harvest-gap:stop-hook-snapshot-volume` node, 2026-10-08 |