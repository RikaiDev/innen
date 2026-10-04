# innen v0.10.0

This release reads each supported agent's actual hook payload, makes a node
retrievable by its own id, and removes three commands that were not earning
their place.

## Changes

### `hook run` reads each agent's wire format

Antigravity does not send `cwd`. Its embedded hook contract states that the
hook's working directory is *the directory containing `hooks.json`*, and every
payload carries camelCase protojson:

```json
{ "conversationId": "…", "workspacePaths": ["/path/to/workspace"],
  "transcriptPath": "…", "artifactDirectoryPath": "…", "modelName": "auto" }
```

So `cwd` resolution silently fell through to a directory that is not a
repository, and every Antigravity turn recorded `branch: nongit`. v0.9.3 made
that safe by refusing to snapshot a non-worktree, which meant Antigravity
produced no snapshots at all. Now:

- work dir precedence is `--cwd` > stdin `workspacePaths[0]` > the existing
  stdin keys > process cwd;
- session aliases include `conversationId`, so an Antigravity turn gets a
  stable per-conversation receipt instead of falling back to the transcript;
- `first_str` accepts a list-valued key where a scalar was expected.

### The opencode adapter no longer discards stdin

`hook install --agent opencode` wrote a plugin that called `hook run` with
`stdio: "ignore"`, so the payload opencode supplied could never be read, and it
passed no session id. Every opencode snapshot therefore had no identity at all.
The adapter now forwards the event on stdin, passes `--session-id`, and
subscribes to `session.idle` rather than `context` — the previous subscription
fired on every model request.

### A node is retrievable by its own id

`node_searchable_text` indexed `label`, `name`, `body`, and `summary` but not
`id`, so the one string guaranteed to identify a node was the one string search
could not find. `harvest-gap:innen-stop-hook-stale-pending-count` was in the
journal and unfindable by id.

The cause was the all-terms admission gate, not the index: a query equal to an
id is split into terms and then matched against label and body, where an id
never appears. A full-id match is now exempt from that gate and scores 1.0. A
*partial* id is not exempt, so the gate cannot be used to smuggle in broad
matches.

### Removed commands

| Removed | Why |
|---|---|
| `cloud` | A canned test stub (`rclone` via `PATH`) shipped in the released binary. |
| `search` | Strictly dominated by `query`: same hits, and `query` was faster while also returning scores, `linked_projects`, and an expansion hint. |
| `lint` | Reported `ok` on every row and exit 0 while `doctor` was failing on a stale index. It was not merely redundant; it masked a real failure. |

`unfinished` and `pickup` were reviewed for redundancy against `resume` and
kept: they carry distinct capability with substantial internal use. The earlier
claim that they duplicated `resume` was wrong.

Visible command count: 28 → 25.

### Structure

`parity.rs` held five unrelated jobs (navigation text, journal counts, journal
order, the project page, and a TOML reader) in one file. It is now
`parity/{guide,status,timeline,project_render,profile}.rs` with `mod.rs`
re-exporting the same public surface, so no call site changed. The profile page
also carried a second hand-rolled TOML parser; `config.rs` and `profile.rs` now
share `toml.rs`.

`scripts/check-structure.sh` is a new CI gate. It fails a source file over 400
lines that has no baseline entry, and fails any internal `Task N` marker left in
the code — the marker check exists because planning vocabulary had leaked into
user-facing `--help` text. Twenty-four files were already over budget when the
gate landed; they are listed in `scripts/structure-baseline.json` as work to
shrink, not as permanent exemptions.

## Verification

Release acceptance requires locked workspace tests, strict Clippy, rustfmt,
the structure gate, a release build, and successful macOS arm64 and Linux
x86_64 CI for the exact release commit. The published archives and metadata must
bind to that commit and match `SHA256SUMS`.

Issues: [#5](https://github.com/RikaiDev/innen/issues/5),
[#6](https://github.com/RikaiDev/innen/issues/6),
[#7](https://github.com/RikaiDev/innen/issues/7)
