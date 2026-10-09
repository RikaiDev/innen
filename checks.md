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
| Release published | pass | `https://github.com/RikaiDev/innen/releases/tag/v0.11.2` | — |

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

## Open problems (not done)

| problem | why not fixed now | where recorded |
|---|---|---|
| `trace::excerpt` byte-offset mismatch | Pre-existing, unrelated to the split. Fixing it changes output for non-ASCII matches; that is a behaviour change and this release promised none. | `docs/release-v0.11.2.md` §Fixed |
| `policy.rs` duplicated 21-line block | Dead code in a file the split did not touch. | `docs/release-v0.11.2.md` §Known issues |
| `task_entry::CandidateItem` unused | Removing it is an API change. | `docs/release-v0.11.2.md` §Known issues |
| `graph/materialize` `typ.parse().unwrap()` | Sound only because `EdgeType::Err = Infallible`. | `docs/release-v0.11.2.md` §Known issues |
| `harvest-check` on `the-mirror`: 455 uncommitted files, last commit 2026-10-06 | Not innen's data and not mine to discard. | `harvest-gap:stop-hook-snapshot-volume` node, 2026-10-08 |