# Vendored dependency patches

Each directory here is an unmodified crates.io source archive except for its
normalized `Cargo.toml`, where one dependency requirement is narrowed or moved
to a patched release. The package version and the upstream Rust implementation
are not relabelled or rewritten. Upstream license and source metadata remain
included. The archived upstream `Cargo.lock` is retained as source provenance;
dependency resolution and security auditing use the repository's root
workspace `Cargo.lock`.

## Tantivy 0.26.1

`tantivy/` is the unmodified crates.io Tantivy **0.26.1** source archive except
for its normalized `Cargo.toml`: `lru = "0.16.3"` is replaced by exact
`lru = "=0.18.4"`.

- Source: https://crates.io/api/v1/crates/tantivy/0.26.1/download
- Archive SHA-256: `edde6a10743fff00a4e1a8c9ef020bf5f3cbad301b7d2d39f2b07f123c4eac07`
- Upstream dependency fix: https://github.com/quickwit-oss/tantivy/pull/3034
- Advisory: https://rustsec.org/advisories/RUSTSEC-2026-0253.html

The fixed lru range starts at 0.18.2. Version 0.18.4 was resolved from the live
registry and pinned on 2026-09-08. Tantivy's published 0.26.1 still requires
lru 0.16; its upstream fixed commit is already part of the 0.27 development
branch with substantial unrelated changes. This narrow manifest patch retains
the tested Tantivy API while removing the unsound dependency.

TODO: remove this patch when a compatible published Tantivy release depends on
patched lru, after the consumer tests and strict advisory audit pass. Keep the
patch and lockfile together. Cargo publishing ignores workspace patches, so a
future crates.io publication must first adopt an upstream/fork dependency that
contains this fix; the current distribution is repository-built binaries.

## tantivy-sstable 0.7.0

`tantivy-sstable/` is the unmodified crates.io tantivy-sstable **0.7.0** source
archive except for its normalized `Cargo.toml`: `zstd = "0.13"` is replaced by
`zstd = "0.14"`. The `experimental` feature request is kept as upstream wrote
it; zstd 0.14 still provides that feature.

- Source: https://crates.io/api/v1/crates/tantivy-sstable/0.7.0/download
- Archive SHA-256: `8a2cfc3ac5164cbadc28965ffb145a8f47582a60ae5897859ad8d4316596c606`
- Advisories: https://rustsec.org/advisories/RUSTSEC-2026-0391.html (zstd),
  RUSTSEC-2026-0396, -0397, -0398, -0399, -0401, -0403 and -0405 (zstd-safe,
  all unsound), RUSTSEC-2026-0410 (zstd-sys)

The advisories were published on 2026-10-16, after the lru patch above was
written. They reached this repository through two transitive paths: jieba-rs
pulls `include-flate-compress`, and tantivy-columnar pulls tantivy-sstable.
The jieba-rs path is closed by a plain upgrade to `include-flate 0.3.5`, whose
`include-flate-compress` already requires zstd 0.14. The tantivy path cannot be
closed by a version requirement alone: tantivy-columnar 0.7 requires
tantivy-sstable 0.7, and upstream tantivy 0.27 moves the whole
sstable/columnar/common family to 0.27 with unrelated behaviour changes.

This narrow manifest patch keeps the tested tantivy-sstable API. The crate
uses only `zstd::bulk::Compressor`, `zstd::bulk::Decompressor` (including
`upper_bound`) and `zstd::zstd_safe::compress_bound`; all three keep their
signatures in zstd 0.14.3, and `zstd-safe 8.2.0` still exports
`compress_bound`. No upstream source line was changed, so the on-disk index
format is untouched.

TODO: remove this patch when a published tantivy release requires zstd 0.14
without moving the surrounding family, after the consumer tests and strict
advisory audit pass. Keep the patch and lockfile together.
