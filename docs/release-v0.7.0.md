# innen v0.7.0

This release adds evidence-gated cleanup for coding-tool conversation history.

## Highlights

- Discover and read sessions from Codex, Claude, OpenCode, agy, and Antigravity stores.
- Keep a 45-day hot window by default and block deletion until extracted knowledge has live provenance to the canonical conversation.
- Revalidate source hashes, byte counts, filesystem identity, and open handles immediately before deletion.
- Treat each session independently during a sweep, so an active or malformed item does not stop unrelated eligible cleanup.
- Delete Antigravity conversation databases, brain directories, and summary rows as one lifecycle.
- Emit bounded summaries by default; detailed per-session output is opt-in.
- Retain knowledge and audit receipts, not raw transcripts or duplicate archives. The retention CLI intentionally has no `--archive` option.

## Verification

- Retention unit tests cover eligibility, changed sources, active sessions, Antigravity lifecycle deletion, and bounded sweep behavior.
- Workspace Clippy passes with warnings denied.
- The CLI rejects the removed `--archive` option.
