# Temporary Tantivy dependency patch

`tantivy/` is the unmodified crates.io Tantivy **0.26.1** source archive except
for its normalized `Cargo.toml`: `lru = "0.16.3"` is replaced by exact
`lru = "=0.18.4"`. The package version and upstream Rust implementation are not
relabelled or rewritten. Upstream license and source metadata remain included.

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
