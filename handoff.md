# handoff — innen structure debt

Date: 2026-10-09
Branch: `main`. Released v0.11.1 (`e19b76a`) and v0.11.2 (`7e67a80`).

## Task

Clear the single-responsibility debt in `innen`, and fix a retrieval defect found
by using the tool. Follow-on from v0.11.0.

## Deliverables

| path | what |
|---|---|
| `scripts/check-structure.py` | gate: 400-line budget, task-marker hygiene, debt-plan completeness |
| `scripts/structure-baseline.json` | 8 entries, **0** with `split_by`, 8 `exempt` |
| `crates/innen-core/src/task_entry.rs` → `task_entry/` | query-term extraction / identity / admission / traversal / entry / render |
| `crates/innen-core/src/{evidence_closure,task_proposal,session_retention,harvest}.rs` → dirs | see `docs/release-v0.11.2.md` |
| `crates/innen-core/src/conversation/{grammar,projection,unfinished}.rs` → dirs | ditto |
| `crates/innen-core/src/trace/source.rs` → `trace/source/` | schema / read_file / read_native / store |
| `docs/release-v0.11.1.md`, `docs/release-v0.11.2.md` | release notes with measured numbers |
| `checks.md` | requirement → evidence table |
| `/Users/gloomcheng/Workspace/innen-wiki/04-index/log.md` | the three findings, appended and committed via the harness |

## Done

- Exact-id retrieval fixed and verified on the released binary.
- Gate now requires every debt entry to name a cut or an exemption; regeneration
  preserves recorded plans instead of overwriting them with `TODO`.
- 17 files split across two releases. Largest non-test file: 1476 → 350.
- 403 tests pass; clippy `-D warnings` clean; `fmt --check` clean.
- Both releases published; Homebrew serving 0.11.2.

## Decisions and why

- **Splitting tests is not a fix.** Six files are exempt because their production
  code is already under budget and the overage is inline tests. Splitting them
  would satisfy a number and change no design.
- **Agents do not run cargo.** Seven agents each triggering a workspace build on a
  machine-wide serialised lock queued for 23 minutes and verified different
  intermediate states of one tree. From v0.11.2 the agents did file surgery only
  and the parent ran one verification.
- **Facade checks must extract names mechanically.** A hand-written list of 31
  names missed `conversation::brief`; only the integration tests caught it. Use
  `git show HEAD:<path>` (not `rtk git show`, which truncated 758 lines to 229 and
  silently dropped `pub fn evaluate`).
- **Line numbers are not cut boundaries.** `grep -nE '^(pub )?(fn|struct|enum)'`
  cannot see `#[derive]`, doc comments or brace pairing. This broke `config.rs`
  seven times and produced a wrong seam map handed to an agent.
- **A classification is a claim that needs checking, not an inference.** The same
  mistake, in nine forms, cost this run most of its wall clock: grouping
  `version` and `PassageScan` as "reached by `trace/tests.rs`" without checking
  each; writing a `split_by` for `session_retention.rs` from a grep outline that
  described two responsibilities already living in `policy.rs` and `sweep.rs`;
  and, writing the `excerpt` regression test, accepting two versions that passed
  against broken code. Each was caught by something other than the check that
  produced it — CI, a compiler, or a standalone probe.
- **An empty result from a command that did not execute is not a pass.** The
  singleflight wrapper refused twice with exit 75 and a one-line diagnostic while
  the machine was below its storage floor; `grep` over that output is empty.
  That empty grep was read as "clippy is clean" and a tag was moved onto a commit
  CI then rejected.

## Open problems

All three code findings from v0.11.2 are fixed in v0.11.3. One remains, and it is
not innen's to close:

`fansee/the-mirror` has 455 uncommitted files, last commit 2026-10-06, idle two
days. Not innen's data to discard; needs its owner.

## Next step

None outstanding in innen. The structure gate is green with no unpaid entries and
the release notes carry no open defects.

If more work arrives, the load-bearing lesson from this run is in the two
`Conversation with the agent` entries below, not in the code.