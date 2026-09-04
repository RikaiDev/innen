//! Append-only journal with validation and quarantine (Task 3).
//!
//! Layout under `<root>/.innen/`:
//! - `journal.jsonl` — one JSON object per line:
//!   `{"id","observed_utc","op","payload"}`.
//! - `journal.lock` — fs2 exclusive-lock sibling, held during
//!   [`Journal::open`] (scan) and rebuild only. Steady-state
//!   [`Journal::append`] relies on `O_APPEND` and takes no lock.
//! - `quarantine/<YYYY-MM-DD>.jsonl` — corrupt lines moved here on open,
//!   one stored line per bad line, UTC date in filename.
//!
//! Validation is minimal per-op shape only (full type checks land in Task 5):
//! `payload` must be an object; `node.upsert`/`node.delete` need string `id`;
//! `edge.assert`/`edge.retract` need strings `from`/`to`/`type`;
//! `config.set` needs string `key`.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Ops accepted by [`Journal::append`]. Full type validation lands in Task 5.
pub const ALLOWED_OPS: &[&str] = &[
    "node.upsert",
    "node.delete",
    "edge.assert",
    "edge.retract",
    "config.set",
];

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("unknown op: {0}")]
    UnknownOp(String),
    #[error("schema violation: {0}")]
    SchemaViolation(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// One validated journal line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub observed_utc: String,
    pub op: String,
    pub payload: Value,
}

/// Field order is the stored line order: id, observed_utc, op, payload.
#[derive(Serialize)]
struct StoredLine<'a> {
    id: &'a str,
    observed_utc: &'a str,
    op: &'a str,
    payload: &'a Value,
}

#[derive(Debug, Clone)]
pub struct Journal {
    pub root: PathBuf,
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`, hand-rolled from `UNIX_EPOCH`
/// (inverse of Howard Hinnant's days-from-civil). No new dependencies.
pub fn observed_utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc(secs)
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    // Civil date from day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{year:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Minimal per-op shape check. Unknown ops are rejected before any disk
/// write; full type validation lands in Task 5.
fn validate(op: &str, payload: &Value) -> Result<(), JournalError> {
    if !ALLOWED_OPS.contains(&op) {
        return Err(JournalError::UnknownOp(op.to_string()));
    }
    let obj = payload.as_object().ok_or_else(|| {
        JournalError::SchemaViolation(format!("{op}: payload must be a JSON object"))
    })?;
    let need = |key: &str| match obj.get(key) {
        Some(Value::String(_)) => Ok(()),
        _ => Err(JournalError::SchemaViolation(format!(
            "{op}: requires string field `{key}`"
        ))),
    };
    match op {
        "node.upsert" | "node.delete" => need("id")?,
        "edge.assert" | "edge.retract" => {
            need("from")?;
            need("to")?;
            need("type")?;
        }
        "config.set" => need("key")?,
        _ => unreachable!("op membership checked above"),
    }
    Ok(())
}

/// Quarantine predicate for `open`: keep lines that parse as JSON objects
/// with string `id` and `op`. Anything else (unparseable, non-object,
/// missing id/op) is moved to quarantine byte-for-byte.
fn keep_line(line: &str) -> bool {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(obj)) => {
            matches!(obj.get("id"), Some(Value::String(_)))
                && matches!(obj.get("op"), Some(Value::String(_)))
        }
        _ => false,
    }
}

impl Journal {
    fn innen_dir(&self) -> PathBuf {
        self.root.join(".innen")
    }

    fn journal_path(&self) -> PathBuf {
        self.innen_dir().join("journal.jsonl")
    }

    /// Open (or create) the journal at `root/.innen`. Corrupt lines are
    /// moved to `quarantine/<UTC-date>.jsonl`; real I/O errors are fatal
    /// and quarantine nothing.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, JournalError> {
        let journal = Self {
            root: root.as_ref().to_path_buf(),
        };
        fs::create_dir_all(journal.innen_dir())?;
        // Exclusive sibling lock for the scan/rebuild window only.
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(journal.innen_dir().join("journal.lock"))?;
        lock_file.lock_exclusive()?;
        let result = journal.quarantine_scan();
        drop(lock_file);
        result?;
        Ok(journal)
    }

    fn quarantine_scan(&self) -> Result<(), JournalError> {
        let content = match fs::read_to_string(self.journal_path()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        let mut good: Vec<&str> = Vec::new();
        let mut bad: Vec<&str> = Vec::new();
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            if keep_line(line) {
                good.push(line);
            } else {
                bad.push(line);
            }
        }
        if bad.is_empty() {
            return Ok(()); // leave a clean file untouched
        }
        let date = &observed_utc_now()[..10];
        let qdir = self.innen_dir().join("quarantine");
        fs::create_dir_all(&qdir)?;
        let qpath = qdir.join(format!("{date}.jsonl"));
        let mut qf = OpenOptions::new().create(true).append(true).open(&qpath)?;
        for line in &bad {
            qf.write_all(line.as_bytes())?;
            qf.write_all(b"\n")?;
            let report = serde_json::json!({
                "quarantined": qpath.to_string_lossy(),
                "reason": "corrupt-line",
                "sha": crate::ids::sha256_hex(line.as_bytes()),
            });
            eprintln!("{report}");
        }
        qf.sync_all()?;
        // Rebuild without the bad lines; rename is atomic in the same dir.
        let tmp_path = self.innen_dir().join("journal.jsonl.tmp");
        {
            let mut tmp = fs::File::create(&tmp_path)?;
            for line in &good {
                tmp.write_all(line.as_bytes())?;
                tmp.write_all(b"\n")?;
            }
            tmp.sync_all()?;
        }
        fs::rename(&tmp_path, self.journal_path())?;
        Ok(())
    }

    /// Validate and append one event; returns its convergent event id.
    /// Pure `O_APPEND` write, no lock (lock is only for open/rebuild).
    pub fn append(&self, op: &str, payload: &Value) -> Result<String, JournalError> {
        validate(op, payload)?;
        let id = crate::ids::event_id(op, payload);
        let now = observed_utc_now();
        let stored = StoredLine {
            id: &id,
            observed_utc: &now,
            op,
            payload,
        };
        let mut bytes = serde_json::to_vec(&stored).expect("journal line serializes");
        bytes.push(b'\n');
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.journal_path())?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        Ok(id)
    }

    /// Read back all valid entries in order. Unparseable tail lines (a
    /// crash that happened after `open`) are skipped; use `open` to
    /// quarantine them.
    pub fn read_all(&self) -> Result<Vec<JournalEntry>, JournalError> {
        let content = match fs::read_to_string(self.journal_path()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            other => other?,
        };
        let mut out = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<JournalEntry>(line) {
                out.push(entry);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open_tmp() -> (tempfile::TempDir, Journal) {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = Journal::open(dir.path()).expect("open");
        (dir, journal)
    }

    #[test]
    fn rejects_unknown_op_without_touching_disk() {
        let (tmp, journal) = open_tmp();
        let journal_path = tmp.path().join(".innen/journal.jsonl");
        let quarantine_dir = tmp.path().join(".innen/quarantine");
        assert!(!journal_path.exists());

        let err = journal.append("bogus.op", &json!({"id": "x"})).unwrap_err();
        assert!(
            matches!(err, JournalError::UnknownOp(_)),
            "unexpected: {err:?}"
        );
        assert!(!journal_path.exists(), "rejected op must not write");
        assert!(!quarantine_dir.exists());

        // Schema violations also write nothing.
        for (op, payload) in [
            ("node.upsert", json!([1, 2])), // non-object payload
            ("node.upsert", json!({"label": "no-id"})),
            ("node.delete", json!({})),
            ("edge.assert", json!({"from": "a", "to": "b"})), // missing type
            ("edge.retract", json!({"from": "a", "to": 1, "type": "t"})),
            ("config.set", json!({"key": 42})),
        ] {
            let err = journal.append(op, &payload).unwrap_err();
            assert!(
                matches!(err, JournalError::SchemaViolation(_)),
                "{op}: unexpected {err:?}"
            );
        }
        assert!(!journal_path.exists());
        assert!(!quarantine_dir.exists());
    }

    #[test]
    fn crash_half_line_quarantined_and_opens_normally() {
        // Hand-rolled UTC clock known answers.
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1767225600), "2026-01-01T00:00:00Z");

        let (tmp, journal) = open_tmp();
        let id = journal
            .append("node.upsert", &json!({"id": "n:1"}))
            .expect("append");

        // Simulate a crash: one garbage line plus one torn tail write.
        let journal_path = tmp.path().join(".innen/journal.jsonl");
        let torn = b"{\"id\": \"n:2\", \"op\": \"node.up";
        {
            let mut f = OpenOptions::new()
                .append(true)
                .open(&journal_path)
                .expect("reopen journal for sabotage");
            f.write_all(b"not json at all\n").expect("write garbage");
            f.write_all(torn).expect("write torn tail");
        }

        // Reopen: bad bytes move to quarantine, good entry survives.
        drop(journal);
        let journal = Journal::open(tmp.path()).expect("reopen");
        let entries = journal.read_all().expect("read");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].op, "node.upsert");

        let today = &observed_utc_now()[..10];
        let qpath = tmp.path().join(format!(".innen/quarantine/{today}.jsonl"));
        let stored = fs::read_to_string(&qpath).expect("quarantine file");
        let lines: Vec<&str> = stored.lines().collect();
        assert_eq!(lines.len(), 2, "one stored line per bad line");
        assert_eq!(lines[0], "not json at all");
        assert_eq!(lines[1], std::str::from_utf8(torn).unwrap());

        // Journal file itself is clean again.
        let journal_text = fs::read_to_string(&journal_path).expect("journal");
        assert_eq!(journal_text.lines().count(), 1);
    }

    #[test]
    fn append_then_readback_roundtrip() {
        let (_tmp, journal) = open_tmp();
        let cases = [
            ("node.upsert", json!({"id": "n:1", "label": "臺"})),
            (
                "edge.assert",
                json!({"from": "n:1", "to": "n:2", "type": "likes"}),
            ),
            ("config.set", json!({"key": "theme", "value": "dark"})),
        ];
        let mut ids = Vec::new();
        for (op, payload) in &cases {
            ids.push(journal.append(op, payload).expect("append"));
        }
        let entries = journal.read_all().expect("read");
        assert_eq!(entries.len(), cases.len());
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.op, cases[i].0);
            assert_eq!(entry.payload, cases[i].1);
            assert_eq!(entry.id, crate::ids::event_id(cases[i].0, &cases[i].1));
            assert_eq!(entry.id, ids[i]);
            assert_eq!(entry.observed_utc.len(), 20);
            assert!(entry.observed_utc.ends_with('Z'));
        }
    }
}
