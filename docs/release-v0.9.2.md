# innen v0.9.2

This release makes retention say when deleting a native session frees no space.

## Changes

- `retention sweep` and `retention purge` measure other hard links to a
  session's native files before deleting. Another tool that hard-links the
  native store keeps those bytes on disk, so the path disappears while free
  space does not change.
  - The sweep summary adds `hard_linked` (sessions) and `hard_linked_bytes`;
    each affected item carries a note. Dry runs report it too, so the
    condition is visible before anything is deleted.
  - The purge receipt adds `other_hard_links` and `hard_linked_bytes`.

Existing fields are unchanged. Hard links are counted on Unix; elsewhere the
new fields are zero.

## Verification

Release acceptance requires locked workspace tests, strict Clippy, rustfmt, a
release build, and successful macOS arm64 and Linux x86_64 CI for the exact
release commit. The published archives and metadata must bind to that commit
and match `SHA256SUMS`.
