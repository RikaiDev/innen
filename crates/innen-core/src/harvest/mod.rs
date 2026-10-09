//! DirectoryTap + harvest check + ingest.
//!
//! Pinned inbox: `<root>/00-inbox/harvest` (flat `*.md`, sorted).
//! Mapping: `id = "source:<sha8(relative_path)>"` (sha8 = first 8 hex of
//! `sha256_hex(relative_path)`); provenance `{path, bytes, sha256,
//! observed_utc}`; `idempotency_key` = full content sha256 hex.
//!
//! The cursor via [`crate::tap`] (`harvest-dir`) records consumed file names,
//! not a count. It used to record a count, which was accepted as a known
//! limitation — "rename/delete shifts counts and misaligns the consumed
//! prefix" — and that misalignment is exactly what silently stranded a whole
//! inbox: once the count passed the listing size, `check` reported nothing new
//! and `ingest` appended nothing, with exit 0. A deletion is normal here,
//! because each pending file asks to be deleted once consumed.
//!
//! Credential scan runs in the caller ([`check`]/[`ingest::run`]) BEFORE any
//! append: a hit skips the file + reports it, never appends. [`TapError`]
//! [`crate::tap::TapError::CredentialHit`] is for direct `Tap` API users only.
//!
//! [`DirectoryTap::dir`] is the KB root (not the inbox dir itself): the inbox
//! is `<dir>/00-inbox/harvest` and the watermark is
//! [`crate::tap::watermark_path`]`(dir, "harvest-dir")`.
//!
//! ## Output contract and edge behavior
//!
//! `HarvestReport` / `IngestReport` / `TapReport` shapes are stable output
//! read by other tools — do not add, rename, or remove fields. New cases
//! must be reported through the existing fields.
//!
//! - IO failures surface as empty: unreadable inbox dir yields an empty
//!   report; [`check`] treats a per-file read failure as empty content;
//!   [`ingest::run`] skips an unreadable file while still advancing the
//!   watermark past it. They are not surfaced as errors because that would
//!   need new fields on the stable structs.
//! - Three UTF-8 policies: `Tap::collect` strict-aborts on non-UTF8
//!   (`TapError::Parse`); [`check`] reads with `unwrap_or_default` so a
//!   non-UTF8 file looks empty (hence clean); [`ingest::run`] scans
//!   `String::from_utf8_lossy`. They stay distinct because unifying them
//!   would change report shapes or the bytes/hashes recorded in reports.
//! - No file-size cap: files are read whole (`fs::read` /
//!   `read_to_string`); 64MB+ files will be slow. A cap would change the
//!   recorded bytes/hashes, so truncation/streaming needs a report-shape
//!   decision first.
//! - check-vs-ingest can disagree on unreadable / non-UTF8 files: check
//!   sees empty which scans clean, while ingest lossy-scans the real
//!   bytes and may skip for credentials. Accepted because the normal
//!   (clean UTF-8) path agrees (see
//!   `check_ingest_agree_on_clean_files`); reconciling the edge cases
//!   needs a report-shape decision first.

pub mod ingest;
mod report;
mod scan;
mod tap;

#[cfg(test)]
mod tests;

pub use report::{HarvestReport, IngestReport, Skipped, TapReport};
pub use scan::check;
pub use tap::{DirectoryTap, TAP_ID};
