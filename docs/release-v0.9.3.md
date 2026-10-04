# innen v0.9.3

This release makes `hook run` refuse to invent a repository, and stops it from
minting a new receipt identity from the clock.

## Changes

- `hook run` only snapshots a Git worktree. A directory that is not inside a
  repository has no repository, no branch and no diff, so the record could
  never carry anything to harvest; it is now skipped with
  `{"skipped":"not-a-repository","dir":…}` instead of being written as
  `repo: <dir>` with `branch: nongit`.
- The snapshot is taken at the worktree root rather than the raw working
  directory, so an agent firing from a subdirectory records the repository.
- When the agent supplies neither a session id nor a transcript path, the
  receipt identity is the worktree state instead of the epoch. Distinct
  worktree states still produce distinct receipts; repeated firings over an
  unchanged state deduplicate to one. Previously each firing minted a fresh
  digest, so an agent without an identity grew the harvest inbox without
  bound.

The guard lives in `hook run` rather than in the generated wiring, so
`hook install` cannot regress it. No shell wrapper is needed for any agent.

`branch` still reads `nongit` for a worktree with an unborn HEAD. After this
gate that value can no longer mean "not a repository".

### Output contract

The deduplicated case is reported as `{"deduped":"no-change","digest":…}`
(TSV `deduped<TAB>no-change`) where it was previously reported under the
`skipped` key. Consumers should read `deduped` or match on the human string,
not on the JSON key name. Decision events (`stop`, `compact`, `pre-compact`)
still print exactly `{"decision":"allow"}` for every outcome, so an agent is
never blocked by this change.

### Accepted information loss

Two identical clean-tree deliveries with no session id and no transcript path
now collapse to one receipt. They were previously distinguished only by
timestamp, and nothing downstream could attribute them individually; the
commits remain in Git history. Every agent that supplies an identity keeps the
existing per-session receipt distinction from 0.8.0.

## Verification

Release acceptance requires locked workspace tests, strict Clippy, rustfmt, a
release build, and successful macOS arm64 and Linux x86_64 CI for the exact
release commit. The published archives and metadata must bind to that commit
and match `SHA256SUMS`.

Issue: <https://github.com/RikaiDev/innen/issues/4>
