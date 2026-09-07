# Changelog

## [0.2.0] - 2026-09-07

### Added

- Read-only conversation adapters for Codex, Claude, Gemini/Antigravity,
  OpenCode, Grok, Copilot, Cursor, VS Code, and related local formats.
- Bounded `resume` and `pickup` continuation briefs with source-line
  provenance, per-agent checkpoints, unresolved evidence, and explicit
  expansion commands.
- Compact and delta response encodings with exact source reconstruction,
  attachment references, pagination, and hash-bound retrieval.
- Project task briefs, evidence views, status history, graph relationships,
  document/revision ledger metadata, and deterministic root configuration.
- Read-only health, lint, source discovery, and unfinished-conversation
  diagnostics.

### Changed

- `query` defaults to `--view context`; use `--view hits` for the legacy hit
  shape and `--view evidence` for expanded evidence.
- `project` defaults to the compact brief; use `project <id> --view full` for
  the legacy full page.
- `resume <session-id>` defaults to the bounded brief; use `--view context`,
  `--view dialogue`, or `--view events` for explicit pages.
- Knowledge-base root resolution is explicit: `--root`, then absolute
  `INNEN_ROOT`, then the registered global root. An unconfigured root fails
  instead of silently using the current directory.

### Fixed

- Preserved source identity, physical line references, parent/child session
  identity, duplicate discovery handling, oversized-record truncation markers,
  and per-author latest checkpoints across continuation and discovery views.
- Kept source retrieval separate from ingestion so reading a conversation does
  not create graph nodes or claim durable knowledge ingestion.

### Release notes

- This release is distributed as platform binaries and source. No crates.io
  publication is implied.
- OpenCode reading requires the `sqlite3` executable on `PATH`; other listed
  adapters use local files directly.
- Verify the executable selected by `PATH` with `command -v innen` and
  `innen --version` when local wrappers and Homebrew installations coexist.
