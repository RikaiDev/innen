//! Shared tantivy schema + CJK tokenizer (Task 7; full index lands in Task 8a).
//!
//! [`schema`] defines the `body` TEXT field indexed with the `"innen-cjk"`
//! tokenizer and the `id` STORED field. [`ensure_tokenizer`] registers the
//! analyzer on an [`tantivy::Index`]'s tokenizer manager; call it once at
//! index build before indexing — the same `Index` then serves searching
//! (registration is idempotent, manager is shared).
//! tantivy 0.24 ships no jieba tokenizer, so [`JiebaTokenizer`] bridges
//! `jieba-rs` (`cut_for_search`, HMM on) into a [`tantivy::tokenizer`]
//! [`Tokenizer`](tantivy::tokenizer::Tokenizer) with jieba char offsets
//! mapped back to byte offsets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tantivy::schema::{Schema, TextFieldIndexing, TextOptions, STORED};
use tantivy::tokenizer::{TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{doc, Index};

use crate::graph::materialize;
use crate::journal::Journal;

/// Tokenizer name referenced by the `body` field's indexing options.
pub const INNEN_CJK: &str = "innen-cjk";

/// jieba-backed tantivy tokenizer behind the `"innen-cjk"` analyzer.
///
/// `Clone` is cheap (shared `Arc` over the loaded dict); `tokenize` uses
/// [`jieba_rs::TokenizeMode::Search`] so CJK queries match sub-word grams
/// the same way at index and query time.
#[derive(Clone, Debug, Default)]
pub struct JiebaTokenizer {
    jieba: Arc<jieba_rs::Jieba>,
}

impl JiebaTokenizer {
    /// Load the embedded jieba dict (no network, no files).
    pub fn new() -> Self {
        Self {
            jieba: Arc::new(jieba_rs::Jieba::new()),
        }
    }
}

/// Pre-tokenized stream produced by [`JiebaTokenizer`].
#[derive(Clone, Debug, Default)]
pub struct JiebaTokenStream {
    tokens: Vec<Token>,
    next_idx: usize,
    current: Token,
}

impl Tokenizer for JiebaTokenizer {
    type TokenStream<'a> = JiebaTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> JiebaTokenStream {
        // jieba reports char offsets; tantivy wants byte offsets.
        let mut byte_of_char: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        byte_of_char.push(text.len());
        let mut tokens = Vec::new();
        for (position, jt) in self
            .jieba
            .tokenize(text, jieba_rs::TokenizeMode::Search, true)
            .iter()
            .enumerate()
        {
            if jt.word.is_empty() {
                continue;
            }
            // Skip out-of-range or degenerate zero-width offsets instead of
            // clamping: emitting a zero-width token would pollute the index.
            let (Some(offset_from), Some(offset_to)) = (
                byte_of_char.get(jt.start).copied(),
                byte_of_char.get(jt.end).copied(),
            ) else {
                continue;
            };
            if offset_from >= offset_to {
                continue;
            }
            tokens.push(Token {
                offset_from,
                offset_to,
                position,
                text: jt.word.to_string(),
                position_length: 1,
            });
        }
        JiebaTokenStream {
            tokens,
            next_idx: 0,
            current: Token::default(),
        }
    }
}

impl TokenStream for JiebaTokenStream {
    fn advance(&mut self) -> bool {
        let Some(next) = self.tokens.get(self.next_idx) else {
            return false;
        };
        self.current = next.clone();
        self.next_idx += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.current
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.current
    }
}

/// Shared schema: `body` TEXT indexed with `"innen-cjk"`, `id` STORED
/// (retrieved from hits, never queried directly; id lookup is the
/// substring seed in [`crate::query`]).
pub fn schema() -> Schema {
    let mut builder = Schema::builder();
    let body_options = TextOptions::default()
        .set_indexing_options(TextFieldIndexing::default().set_tokenizer(INNEN_CJK));
    builder.add_text_field("body", body_options);
    builder.add_text_field("id", STORED);
    builder.build()
}

/// Register (idempotent) the `"innen-cjk"` analyzer on `index`.
/// Call once at index build before indexing; searching reuses the same manager.
pub fn ensure_tokenizer(index: &tantivy::Index) {
    index
        .tokenizers()
        .register(INNEN_CJK, TextAnalyzer::from(JiebaTokenizer::new()));
}

/// Derived index layout under `<root>/.innen/` (Task 8a; the journal stays
/// the source of truth):
///
/// - `index.redb` — redb with `events(id → raw entry JSON)`,
///   `nodes(id → node JSON)`, `edges((type, from) → JSON list of edge rows)`.
///   Edge keys join the canonical edge name and from-id with `\0`; each row
///   keeps `from`/`type`/`to`/`weight`/`valid_from`/`valid_until`/`retracted`/
///   `observed_utc` (retracted rows retained, validity unfiltered — readers
///   filter per query).
/// - `fts/` — on-disk tantivy index over materialized nodes with [`schema`]
///   + [`ensure_tokenizer`] (same contract as query's throwaway in-RAM
///     index: text is `label`, or `label + " " + body`; docs added in sorted
///     id order).
const EVENTS: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("events");
const NODES: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("nodes");
const EDGES: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("edges");

/// Derived-index build failure: journal I/O, filesystem I/O, tantivy, redb.
/// Corrupt `index.redb`/`fts` bytes surface here; recover via [`rebuild`].
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("journal: {0}")]
    Journal(#[from] crate::journal::JournalError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tantivy: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("redb: {0}")]
    Redb(#[from] redb::Error),
}

// redb method errors convert into `redb::Error` (one step the `#[from]`
// above cannot chain through `?` on its own); forward them so every redb
// call site keeps plain `?` while the enum stays four variants.
macro_rules! forward_redb {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for IndexError {
                fn from(e: $ty) -> Self {
                    Self::Redb(e.into())
                }
            }
        )*
    };
}

forward_redb!(
    redb::CommitError,
    redb::DatabaseError,
    redb::StorageError,
    redb::TableError,
    redb::TransactionError
);

fn redb_path(root: &Path) -> PathBuf {
    root.join(".innen").join("index.redb")
}

fn fts_path(root: &Path) -> PathBuf {
    root.join(".innen").join("fts")
}

/// Full replay over the journal into redb + tantivy (idempotent rerun:
/// redb keys are overwritten and the FTS segment is cleared before
/// re-adding, so a second `build` converges to the same state).
///
/// Replay is unfiltered (`as_of = None`, `include_expired = true`): history
/// and retracted rows are preserved in the derived files; per-query
/// filtering stays with the reader.
///
/// The write is two-phase and non-atomic (redb commit, then tantivy commit);
/// a crash in between can leave a new redb beside a stale `fts/` until the
/// next build. An existing `fts/` is reused via `open_in_dir`, falling back
/// to `create` only when open fails — [`rebuild`] (which removes `fts/`
/// first) is the reliable recovery path for corrupt derived bytes.
pub fn build(root: &Path) -> Result<(), IndexError> {
    std::fs::create_dir_all(root.join(".innen"))?;
    std::fs::create_dir_all(fts_path(root))?;

    let journal = Journal::open(root)?;
    let entries = journal.read_all()?;
    let events: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "op": e.op,
                "payload": e.payload,
                "observed_utc": e.observed_utc,
            })
        })
        .collect();
    let materialized = materialize(&events, None, true);

    // --- redb (one write txn; journal ops are monotonic in keys, so
    // --- overwrite converges; full reset is `rebuild`).
    let db = redb::Database::create(redb_path(root))?;
    {
        let txn = db.begin_write()?;
        {
            let mut table = txn.open_table(EVENTS)?;
            for entry in &entries {
                let raw = serde_json::to_string(entry).expect("journal entry serializes");
                table.insert(entry.id.as_str(), raw.as_str())?;
            }
        }
        {
            let mut table = txn.open_table(NODES)?;
            for (id, node) in &materialized.nodes {
                let raw = serde_json::to_string(node).expect("node serializes");
                table.insert(id.as_str(), raw.as_str())?;
            }
        }
        {
            let mut grouped: BTreeMap<(String, String), Vec<serde_json::Value>> = BTreeMap::new();
            for edge in &materialized.edges {
                grouped
                    .entry((edge.edge.to_string(), edge.from.clone()))
                    .or_default()
                    .push(serde_json::json!({
                        "from": edge.from,
                        "type": edge.edge.to_string(),
                        "to": edge.to,
                        "weight": edge.weight,
                        "valid_from": edge.valid_from,
                        "valid_until": edge.valid_until,
                        "retracted": edge.retracted,
                        "observed_utc": edge.observed_utc,
                    }));
            }
            let mut table = txn.open_table(EDGES)?;
            for ((ty, from), list) in &grouped {
                let key = format!("{ty}\0{from}");
                let raw = serde_json::to_string(list).expect("edge list serializes");
                table.insert(key.as_str(), raw.as_str())?;
            }
        }
        txn.commit()?;
    }

    // --- tantivy (open-or-create; clear, then reindex in sorted id order).
    let index_schema = schema();
    let body_field = index_schema.get_field("body").expect("schema defines body");
    let id_field = index_schema.get_field("id").expect("schema defines id");
    let fts = fts_path(root);
    let index = match Index::open_in_dir(&fts) {
        Ok(index) => index,
        Err(_) => Index::create_in_dir(&fts, index_schema)?,
    };
    ensure_tokenizer(&index);
    {
        let mut writer = index.writer(15_000_000)?;
        writer.delete_all_documents()?;
        let mut ids: Vec<&String> = materialized.nodes.keys().collect();
        ids.sort();
        for id in ids {
            let node = &materialized.nodes[id];
            let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let body = node.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let text = if body.is_empty() {
                label.to_string()
            } else if label.is_empty() {
                body.to_string()
            } else {
                format!("{label} {body}")
            };
            writer.add_document(doc!(
                body_field => text,
                id_field => id.clone(),
            ))?;
        }
        writer.commit()?;
    }
    Ok(())
}

/// Drop derived index files (`index.redb` + `fts/`, missing files are fine)
/// and [`build`] again from the journal (untouched — it is the source of
/// truth). This is the recovery path for corrupt derived bytes.
///
/// A crash between the remove step and the end of [`build`] leaves total
/// derived loss (no `index.redb`, no `fts/`) until the next build completes;
/// this is acceptable because the journal is the source of truth and the next
/// build restores everything.
///
/// The build itself is two-phase and non-atomic: the redb transaction commits
/// first, then the tantivy writer commits. A crash in between can leave a new
/// redb beside a stale `fts/` until the next build converges them.
///
/// `build` reuses an existing `fts/` via `open_in_dir`, falling back to
/// `create` only when open fails; a corrupt-but-openable `fts/` may not
/// self-heal. Removing `fts/` first (as done here) is the reliable recovery
/// path.
pub fn rebuild(root: &Path) -> Result<(), IndexError> {
    match std::fs::remove_file(redb_path(root)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    match std::fs::remove_dir_all(fts_path(root)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    build(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::journal::Journal;
    use crate::query::{query, QueryParams};

    /// Fixture journal: 3 node.upsert + 1 edge.assert (`t:1 -FOLLOWS_UP-> t:2`,
    /// a core adjacency row so query BFS traverses it).
    fn fixture_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = Journal::open(dir.path()).expect("open");
        for (id, ty, label) in [
            ("t:1", "Task", "alpha one"),
            ("t:2", "Task", "alpha two"),
            ("p:1", "Project", "alpha project"),
        ] {
            journal
                .append(
                    "node.upsert",
                    &serde_json::json!({"id": id, "type": ty, "label": label}),
                )
                .expect("append node");
        }
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": "t:1",
                    "to": "t:2",
                    "type": "FOLLOWS_UP",
                    "valid_from": "2026-01-01T00:00:00Z",
                }),
            )
            .expect("append edge");
        dir
    }

    /// Query `t:1` (exact id-substring seed); the edge shows up as `t:2`
    /// reached at depth 1 with `why: "graph"`.
    fn query_ids(root: &Path) -> Vec<crate::query::Hit> {
        query(
            root,
            &QueryParams {
                q: "t:1".to_string(),
                as_of: None,
                limit: 20,
                include_expired: false,
            },
        )
        .expect("query")
        .hits
    }

    /// Total edge rows across every `(type, from)` list in the derived redb.
    /// Reads the `edges` table as a black box (own handle, JSON-list values).
    /// Every row read propagates via `expect`: a corrupt row fails the test
    /// instead of being silently skipped.
    fn redb_edge_count(root: &Path) -> usize {
        use redb::ReadableTable as _;
        let db = redb::Database::open(redb_path(root)).expect("open redb");
        let txn = db.begin_read().expect("read txn");
        let table = txn.open_table(EDGES).expect("edges table");
        let mut total = 0;
        for row in table.iter().expect("iter") {
            let (_, v) = row.expect("edge row readable");
            let list =
                serde_json::from_str::<Vec<serde_json::Value>>(v.value()).expect("edge list value");
            total += list.len();
        }
        total
    }

    /// `(events rows, nodes rows, total edge rows)` in the derived redb.
    /// All reads propagate via `expect` (see [`redb_edge_count`]).
    fn redb_counts(root: &Path) -> (u64, u64, usize) {
        use redb::{ReadableTable as _, ReadableTableMetadata as _};
        let db = redb::Database::open(redb_path(root)).expect("open redb");
        let txn = db.begin_read().expect("read txn");
        let events = txn
            .open_table(EVENTS)
            .expect("events table")
            .len()
            .expect("events len");
        let nodes = txn
            .open_table(NODES)
            .expect("nodes table")
            .len()
            .expect("nodes len");
        let edges_table = txn.open_table(EDGES).expect("edges table");
        let mut edge_rows = 0;
        for row in edges_table.iter().expect("iter") {
            let (_, v) = row.expect("edge row readable");
            let list =
                serde_json::from_str::<Vec<serde_json::Value>>(v.value()).expect("edge list value");
            edge_rows += list.len();
        }
        (events, nodes, edge_rows)
    }

    /// Direct tantivy read-back from the built `fts/` dir (not the query
    /// throwaway): open the on-disk index, register the tokenizer, run `q`,
    /// return sorted `id`s. Panics on I/O/tantivy failure, so corruption
    /// surfaces as a test failure rather than an empty result.
    fn fts_ids(root: &Path, q: &str) -> Vec<String> {
        use tantivy::collector::TopDocs;
        use tantivy::query::QueryParser;
        use tantivy::schema::Value as _;
        let index_schema = schema();
        let body_field = index_schema.get_field("body").expect("schema defines body");
        let id_field = index_schema.get_field("id").expect("schema defines id");
        let index = tantivy::Index::open_in_dir(fts_path(root)).expect("open fts");
        ensure_tokenizer(&index);
        let reader = index.reader().expect("fts reader");
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(&index, vec![body_field]);
        let parsed = parser.parse_query(q).expect("parse fts query");
        let top = searcher
            .search(&parsed, &TopDocs::with_limit(20))
            .expect("fts search");
        let mut ids = Vec::new();
        for (_, addr) in top {
            let fts_doc: tantivy::TantivyDocument = searcher.doc(addr).expect("fts doc");
            if let Some(id) = fts_doc.get_first(id_field).and_then(|v| v.as_str()) {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        ids
    }

    /// True only if a direct `fts/` read finds `expect_id` for `q`; any
    /// open/parse/search error returns false. Used to prove FTS corruption
    /// actually broke read-back before rebuild.
    fn fts_read_finds(root: &Path, q: &str, expect_id: &str) -> bool {
        use tantivy::collector::TopDocs;
        use tantivy::query::QueryParser;
        use tantivy::schema::Value as _;
        let index_schema = schema();
        let Ok(body_field) = index_schema.get_field("body") else {
            return false;
        };
        let Ok(id_field) = index_schema.get_field("id") else {
            return false;
        };
        let Ok(index) = tantivy::Index::open_in_dir(fts_path(root)) else {
            return false;
        };
        ensure_tokenizer(&index);
        let Ok(reader) = index.reader() else {
            return false;
        };
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(&index, vec![body_field]);
        let Ok(parsed) = parser.parse_query(q) else {
            return false;
        };
        let Ok(top) = searcher.search(&parsed, &TopDocs::with_limit(20)) else {
            return false;
        };
        for (_, addr) in top {
            if let Ok(fts_doc) = searcher.doc::<tantivy::TantivyDocument>(addr) {
                if fts_doc.get_first(id_field).and_then(|v| v.as_str()) == Some(expect_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Overwrite every regular file under `fts/` with garbage. `rebuild`
    /// removes the whole dir, so it still recovers.
    fn corrupt_fts_files(root: &Path) {
        let mut corrupted = 0;
        let mut stack = vec![fts_path(root)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read fts dir") {
                let path = entry.expect("fts entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    std::fs::write(&path, b"garbage-not-a-tantivy-file").expect("corrupt fts file");
                    corrupted += 1;
                }
            }
        }
        assert!(corrupted > 0, "expected fts files to corrupt");
    }

    #[test]
    fn index_corrupt_recovers() {
        let dir = fixture_root();
        build(dir.path()).expect("build");

        // Query sees the 1 edge: seed t:1 reaches t:2 at depth 1.
        let hits = query_ids(dir.path());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].node_id, "t:1");
        assert_eq!(hits[1].node_id, "t:2");
        assert_eq!(hits[1].why, "graph");
        assert_eq!(hits[1].score, 0.5);
        assert_eq!(redb_edge_count(dir.path()), 1);
        // Direct tantivy read-back proves FTS derived state (not just redb).
        let fts_alpha = fts_ids(dir.path(), "alpha");
        assert!(
            fts_alpha.contains(&"t:1".to_string()),
            "fts must index t:1, got {fts_alpha:?}"
        );
        assert!(
            fts_alpha.contains(&"t:2".to_string()),
            "fts must index t:2, got {fts_alpha:?}"
        );
        assert!(
            fts_alpha.contains(&"p:1".to_string()),
            "fts must index p:1, got {fts_alpha:?}"
        );

        // Corrupt index.redb bytes (overwrite with garbage).
        let redb_file = redb_path(dir.path());
        std::fs::write(&redb_file, b"garbage-not-a-redb-file").expect("corrupt");
        assert!(
            redb::Database::open(&redb_file).is_err(),
            "corruption must break redb open"
        );

        // rebuild() recovers: the edge is visible again via query, redb,
        // and direct FTS read-back.
        rebuild(dir.path()).expect("rebuild");
        let hits = query_ids(dir.path());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].node_id, "t:1");
        assert_eq!(hits[1].node_id, "t:2");
        assert_eq!(hits[1].why, "graph");
        assert_eq!(hits[1].score, 0.5);
        assert_eq!(redb_edge_count(dir.path()), 1);
        let fts_alpha = fts_ids(dir.path(), "alpha");
        assert!(
            fts_alpha.contains(&"t:1".to_string()),
            "fts must recover t:1, got {fts_alpha:?}"
        );
        assert!(
            fts_alpha.contains(&"t:2".to_string()),
            "fts must recover t:2, got {fts_alpha:?}"
        );
        assert!(
            fts_alpha.contains(&"p:1".to_string()),
            "fts must recover p:1, got {fts_alpha:?}"
        );

        // Corrupt FTS files (garbage into every file under fts/).
        corrupt_fts_files(dir.path());
        assert!(
            !fts_read_finds(dir.path(), "alpha", "t:1"),
            "fts corruption must break direct read-back"
        );

        // rebuild() recovers FTS as well.
        rebuild(dir.path()).expect("rebuild after fts corruption");
        let fts_alpha = fts_ids(dir.path(), "alpha");
        assert!(
            fts_alpha.contains(&"t:1".to_string()),
            "fts must recover t:1 after fts corruption, got {fts_alpha:?}"
        );
        assert!(
            fts_alpha.contains(&"t:2".to_string()),
            "fts must recover t:2 after fts corruption, got {fts_alpha:?}"
        );
        assert!(
            fts_alpha.contains(&"p:1".to_string()),
            "fts must recover p:1 after fts corruption, got {fts_alpha:?}"
        );
        let hits = query_ids(dir.path());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].node_id, "t:1");
        assert_eq!(hits[1].node_id, "t:2");
        assert_eq!(redb_edge_count(dir.path()), 1);
    }

    #[test]
    fn build_is_idempotent() {
        let dir = fixture_root();
        build(dir.path()).expect("first build");
        let counts_first = redb_counts(dir.path());
        let hits_first = query_ids(dir.path());
        let fts_first = fts_ids(dir.path(), "alpha");
        build(dir.path()).expect("second build");
        let counts_second = redb_counts(dir.path());
        let hits_second = query_ids(dir.path());
        let fts_second = fts_ids(dir.path(), "alpha");
        assert_eq!(
            counts_first, counts_second,
            "second build must converge to same row counts"
        );
        assert_eq!(
            hits_first, hits_second,
            "second build must converge to same query hits"
        );
        assert_eq!(
            fts_first, fts_second,
            "second build must converge to same fts ids"
        );
        // Fixture pins the converged state: 4 events, 3 nodes, 1 edge row.
        assert_eq!(counts_first, (4, 3, 1));
    }

    #[test]
    fn rebuild_with_missing_files() {
        let dir = fixture_root();
        // No derived files yet: rebuild must still work (missing files fine).
        assert!(!redb_path(dir.path()).exists());
        assert!(!fts_path(dir.path()).exists());
        rebuild(dir.path()).expect("rebuild with missing files");
        let hits = query_ids(dir.path());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].node_id, "t:1");
        assert_eq!(hits[1].node_id, "t:2");
        assert_eq!(redb_edge_count(dir.path()), 1);
        assert!(fts_read_finds(dir.path(), "alpha", "t:1"));
    }

    #[test]
    fn empty_journal_builds_empty_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        Journal::open(dir.path()).expect("open");
        build(dir.path()).expect("build empty");
        assert_eq!(redb_counts(dir.path()), (0, 0, 0));
        assert!(
            fts_ids(dir.path(), "alpha").is_empty(),
            "empty journal must build empty fts"
        );
        assert!(
            query_ids(dir.path()).is_empty(),
            "empty journal must query empty"
        );
    }
}
