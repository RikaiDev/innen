# innen v0.4.0

Release notes, upgrade behavior, and the verification process for this version.

## Release notes

Wiki source references can now be projected into the graph with `innen wiki sync`.
`innen trace --q ...` follows bounded provenance paths to source passages, and
returns hash-checked `expand_argv` commands for recovering exact records. Explicit
session descriptors in selected evidence files can continue retrieval into native
agent conversations. Coverage limits and missing sources remain visible.

This release also adds persistent task-history context and middleware commands,
and rejects graph writes with missing endpoints by default. The CLI and retrieval
implementation are separated into Rust modules by responsibility; existing CLI
commands remain available.

The dependency fix for RUSTSEC-2026-0253 retains Tantivy 0.26.1 and pins its lru
dependency to 0.18.4. The vendored source differs only in that dependency manifest;
see [the patch provenance and removal conditions](../vendor/README.md). CI now
rejects advisory warnings as well as vulnerabilities.

## Upgrade behavior and limits

- After wiki edits, run `innen wiki sync` explicitly. Raw sources and graph history
  are preserved; projection changes retract only relationships owned by the sync.
- Trace caches are derived local state and rebuild when their schema changes.
  Exact expansion verifies record bytes even when search reused a cache.
- History middleware remains opt-in. Upgrading a binary does not grant hook trust
  or replace a previously pinned hook executable.
- The bounded OpenCode scanner is not implemented. Remote URLs are not fetched
  automatically. Missing provenance and exhausted budgets are not proof of absence.
- No universal retrieval-quality, latency, token, or subscription-savings claim is
  made. A successful bounded search does not establish complete source coverage.

## Publication sequence

1. Review and commit the intended source, tests, docs, lockfile, and vendored patch
   together. Exclude private journals, transcripts, caches, and local research
   receipts. Existing unrelated changes must not enter the release by accident.
2. Push the reviewed candidate and require the entire Rust CI run for that exact
   commit to pass: formatting, strict Clippy, all-feature tests, security audit,
   and optimized macOS arm64 / Linux x86_64 package checks. Local macOS evidence
   alone does not establish Linux acceptance.
3. Download packages and build metadata from that successful run. Verify each
   metadata source_commit equals the candidate commit, version is 0.4.0, target
   and archive agree, and the archive SHA-256 matches. Generate SHA256SUMS from
   those exact archive bytes. Do not substitute locally rebuilt binaries.
4. Prepare a draft GitHub Release for v0.4.0 targeting that commit, with both
   archives, per-target build metadata, SHA256SUMS, and these reviewed release
   notes. Verify the tag target and all draft assets before publishing.
5. Publish only after release authorization. Verify downloadable assets against
   SHA256SUMS, then confirm the Homebrew update and installed binary version.
   If the tap token is absent, perform an explicitly authorized manual tap update
   from the same verified asset hash; do not report Homebrew completion early.

The current workspace patch is suitable for repository-built binaries. Cargo
publishing ignores workspace patches, so this candidate is **not a crates.io
publication candidate** until the fixed dependency is carried by a publishable
upstream/fork dependency.

## Acceptance state

The preceding implementation passed a full workspace test run after the security
fix, followed by 29 focused tests for the final retrieval changes and strict
workspace/all-target/all-feature Clippy. The previous 22 CLI help snapshots match
after normalizing only the copied executable basename. Advisory audit reported
zero vulnerabilities and warnings with no ignored advisories.

The local 0.4.0 candidate passed `cargo test --locked --workspace --all-features`
with exit 0. Both workspace packages resolve to 0.4.0 under locked/offline Cargo
metadata, and the built executable reports `innen 0.4.0`. The exact new CI audit
command, `cargo audit --file Cargo.lock --deny warnings`, passed with zero
vulnerabilities, zero warnings, and no ignored advisories. Workflow YAML parsing
and diff whitespace checks passed; actionlint was not installed, and the new
workflow execution is verified separately for the release commit.

Local checks and remote CI are separate evidence. Publication requires the exact
candidate commit to pass the gates above. The release asset metadata records the
source commit and workflow run so consumers can verify the published build.
