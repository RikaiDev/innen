# innen v0.8.0

This release makes completed Codex document deliveries recoverable when they were
not added to the knowledge graph.

## Changes

- `innen query --q ...` searches document filenames in `~/Documents` and
  `~/Downloads` when graph resolution is missing. Results are labelled
  `unindexed_local_candidates`; scan limits and incomplete coverage are explicit.
- `innen harvest --check --source codex --session <id>` reads final-answer document
  links from one native Codex session and reports source lines, current path
  existence, and bounded moved-file candidates.
- Hook receipts now include session and event identity, so a clean Git worktree
  does not suppress another session's pending extraction receipt.
- Multi-term graph queries require all meaningful request terms before a literal
  match can admit a candidate.

Local candidates remain read-only evidence. The command does not archive document
bytes or infer which linked file is the approved delivery. Use `artifact add`
after checking the file and project identity. The delivery audit currently
supports Codex final-answer links in Markdown with absolute document paths.

## Verification

Release acceptance requires locked workspace tests, strict Clippy, advisory
audit, a release build, and successful macOS arm64 and Linux x86_64 CI for the
exact release commit. The published archives and metadata must bind to that
commit and match `SHA256SUMS`.
