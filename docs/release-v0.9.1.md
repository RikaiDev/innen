# innen v0.9.1

This release makes `retention sweep` finish in minutes instead of hours.

## Changes

- `retention sweep` enumerates the native store once. v0.9.0 re-enumerated it
  three times for every swept session, and each enumeration reads the last
  1 MiB of every native session, so a 1,000-session sweep took about 13 s per
  session. A single `retention purge` now enumerates once instead of twice.
- The Homebrew formula update runs after the release assets exist
  (`release-assets.yml` calls `update-homebrew.yml`), instead of racing them.
- `scripts/post-release.sh` retires a dev build made from a dirty tree once
  the tree is clean and HEAD is contained in the release tag.

No command, flag or output schema changed.

## Verification

Release acceptance requires locked workspace tests, strict Clippy, rustfmt, a
release build, and successful macOS arm64 and Linux x86_64 CI for the exact
release commit. The published archives and metadata must bind to that commit
and match `SHA256SUMS`.
