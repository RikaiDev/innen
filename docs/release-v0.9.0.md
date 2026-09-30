# innen v0.9.0

This release lets native coding sessions be cleared without an agent rereading
them, keeps one current release per deliverable, and stops dev builds from
shadowing Homebrew after a release.

## Changes

- Retention keeps a 7-day hot window by default (was 45). `--retention-days N`
  sets another window and `--no-retention` keeps none, on `retention plan`,
  `purge`, `sweep` and `harvest --coding-sessions`. The live-session and proof
  gates still apply.
- `retention sweep` compacts closed sessions whose only blocker is a missing
  extraction: it keeps the user/assistant dialogue with original source line
  numbers under `00-inbox/harvest/compact/`, records it as a `CompactTranscript`
  knowledge node with a retention proof, then purges through the existing
  gates. Dry runs report these sessions as `compactable`.
- Codex subagent sessions are recognised from `source.subagent.thread_spawn`.
  Sweep handles children before parents, and purge refuses a Codex parent while
  any child session still exists, because `codex delete` also removes them.
- Codex and OpenCode deletion is refused under `--source-root`, since their
  CLIs act on the canonical store; every deletion is verified on disk before a
  receipt reports it.
- `artifact finalize --plan <json> [--apply]` publishes explicit final files to
  `03-output/artifacts/collections/<name>/` and removes only the hash-bound
  sources and superseded files listed in the plan.
- `scripts/dev-install.sh` installs a dev build with a provenance marker;
  `scripts/post-release.sh vX.Y.Z` retires it once the release contains its
  commit. A clean dev build older than the installed Homebrew release removes
  itself on its next run and hands the command to the Homebrew binary.

## Verification

Release acceptance requires locked workspace tests, strict Clippy, rustfmt, a
release build, and successful macOS arm64 and Linux x86_64 CI for the exact
release commit. The published archives and metadata must bind to that commit
and match `SHA256SUMS`.
