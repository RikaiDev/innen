# Changelog

## [0.4.0] - 2026-09-11

### Added

- Agent stop hooks: `innen hook install|run|pending` for
  claude-code, codex, opencode, antigravity, and grok. Session end
  snapshots the worktree into `00-inbox/harvest/pending-*.md`
  (digest-deduped, fail-open); default install scope is user-global.

### Added

- Opt-in native conversation grammar with reversible, hash-bound packets.
- Contract-scoped evidence closure, evidence IDs, reviewed premise state, and
  exact support/counterevidence spans.
- External task-proposal validation that keeps source matching, semantic
  review, and domain authorization separate.
- Metadata-only `artifact add-tree` inventories with bounded-memory hashing,
  deterministic manifests, and symlink-safe traversal.
- One-shot project resolution from canonical IDs, namespace-free slugs,
  unique labels/basenames, and absolute Workspace paths.

### Changed

- Project brief, evidence, and full views now share one identity resolver;
  ambiguous shorthand lists candidates instead of guessing.
- Accepted proposal reviews must identify atomic supported, missing, or
  refuted premises. Missing evidence remains unknown, and explicit refutation
  cannot be erased by added support.

### Fixed

- Continuation integration fixtures now use the execution date for tests of
  the default recent-session window; the explicit old-session boundary test
  remains fixed to historical input.

### Verification

- 249 local workspace/all-target/all-feature tests passed with no failures or
  ignored tests; strict Clippy and locked release build passed.
- GitHub CI passed on macOS aarch64 and Ubuntu x86_64 for commit `031dbc9`.
- Output-length research remains historical and is not a provider-usage,
  billing, comprehension, or current-version performance claim.

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
