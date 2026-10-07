//! Exact-identity discovery; no full-text search or ingestion side effects.
use super::ReadError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Antigravity,
    Codex,
    Claude,
    Gemini,
    Opencode,
    Grok,
    Copilot,
    Cursor,
    Vscode,
    Qwen,
}

impl Source {
    #[allow(non_upper_case_globals)]
    pub const Agy: Source = Source::Antigravity;
    pub const AGY: Source = Source::Antigravity;

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Opencode => "opencode",
            Self::Grok => "grok",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::Vscode => "vscode",
            Self::Qwen => "qwen",
        }
    }

    pub fn parse(name: &str) -> Result<Self, ReadError> {
        match name {
            "agy" | "antigravity" => Ok(Self::Antigravity),
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "opencode" => Ok(Self::Opencode),
            "grok" => Ok(Self::Grok),
            "copilot" => Ok(Self::Copilot),
            "cursor" => Ok(Self::Cursor),
            "vscode" => Ok(Self::Vscode),
            "qwen" => Ok(Self::Qwen),
            _ => Err(ReadError(format!("unknown source: {name}"))),
        }
    }
}

pub struct Located {
    pub source: Source,
    pub path: PathBuf,
}

pub fn validate_id(id: &str) -> Result<String, ReadError> {
    let uuid = id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if uuid {
        return Ok(id.to_ascii_lowercase());
    }
    if id.starts_with("ses_")
        && (5..=128).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Ok(id.into());
    }
    Err(ReadError(
        "invalid UUID or native session ID: expected a full UUID or OpenCode ses_... ID".into(),
    ))
}

pub(crate) fn default_stores() -> Result<Vec<(Source, PathBuf)>, ReadError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| ReadError("HOME unavailable; use --source and --source-root".into()))?;
    let home = PathBuf::from(home);
    let mut stores = vec![
        (Source::Codex, home.join(".codex/sessions")),
        (Source::Codex, home.join(".codex/archived_sessions")),
        (Source::Claude, home.join(".claude/projects")),
        (Source::Qwen, home.join(".qwen/projects")),
        (Source::Gemini, home.join(".gemini/tmp")),
        (Source::Grok, home.join(".grok/sessions")),
        (Source::Copilot, home.join(".copilot/session-state")),
        (Source::Cursor, home.join(".cursor/projects")),
        (
            Source::Opencode,
            home.join(".local/share/opencode/opencode.db"),
        ),
    ];
    for name in ["antigravity-cli", "antigravity", "antigravity-ide"] {
        stores.push((
            Source::Antigravity,
            home.join(".gemini").join(name).join("brain"),
        ));
    }
    for app in ["Code", "Code - Insiders", "VSCodium"] {
        stores.push((
            Source::Vscode,
            home.join("Library/Application Support")
                .join(app)
                .join("User/workspaceStorage"),
        ));
        stores.push((
            Source::Vscode,
            home.join(".config").join(app).join("User/workspaceStorage"),
        ));
    }
    Ok(stores)
}

pub fn locate(root: Option<&Path>, source: &str, id: &str) -> Result<Located, ReadError> {
    let filter = if source == "auto" {
        None
    } else {
        Some(Source::parse(source)?)
    };
    let stores = if let Some(root) = root {
        vec![(
            filter.ok_or_else(|| ReadError("--source-root requires --source <tool>".into()))?,
            root.to_owned(),
        )]
    } else {
        default_stores()?
    };
    let mut found: Vec<Located> = Vec::new();
    for (kind, root) in stores {
        if filter.is_some_and(|s| s != kind) {
            continue;
        }
        let candidates = if kind == Source::Antigravity {
            let mut list = vec![
                root.join(id)
                    .join(".system_generated/logs/transcript.jsonl"),
                root.join(id).join("transcript.jsonl"),
                root.join(format!("{id}.jsonl")),
            ];
            let found_direct = list.iter().any(|p| p.try_exists().unwrap_or(false));
            if !found_direct {
                let _ = walk(&root, kind, id, 6, &mut list);
            }
            list
        } else if kind == Source::Copilot {
            vec![root.join(id).join("events.jsonl")]
        } else if kind == Source::Opencode {
            let path = if root.is_dir() {
                root.join("opencode.db")
            } else {
                root
            };
            if path.try_exists().map_err(|e| err(&path, e))?
                && !sqlite(
                    &path,
                    &format!(
                        "SELECT json_object('id',id) FROM {} WHERE id='{}' LIMIT 1",
                        session_table(&path)?,
                        id
                    ),
                )?
                .is_empty()
            {
                vec![path]
            } else {
                Vec::new()
            }
        } else {
            let mut candidates = Vec::new();
            walk(&root, kind, id, 6, &mut candidates)?;
            candidates
        };
        for path in candidates {
            match path.canonicalize() {
                Ok(path) => {
                    if !found.iter().any(|f| f.path == path && f.source == kind) {
                        found.push(Located { source: kind, path });
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(err(&path, e)),
            }
        }
    }
    match found.len() {
        0 => Err(ReadError(format!("conversation not found: {id} (source {source}); use --source <tool> --source-root <store> for relocated data; no ingest required"))),
        1 => Ok(found.remove(0)),
        _ => Err(ReadError(format!("ambiguous conversation: {id}; select --source and --source-root from: {}", found.iter().map(|f| format!("{:?}:{}", f.source, f.path.display())).collect::<Vec<_>>().join(", ")))),
    }
}

fn walk(
    root: &Path,
    source: Source,
    id: &str,
    depth: usize,
    found: &mut Vec<PathBuf>,
) -> Result<(), ReadError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(err(root, e)),
    };
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(root, e))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| err(&path, e))?;
        if ty.is_dir() && depth > 0 {
            walk(&path, source, id, depth - 1, found)?;
        }
        if !ty.is_file() {
            continue;
        }
        let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
        let matched = match source {
            Source::Codex => ext == "jsonl" && (stem == id || stem.ends_with(&format!("-{id}"))),
            Source::Claude | Source::Cursor | Source::Qwen => ext == "jsonl" && stem == id,
            Source::Antigravity => {
                (ext == "jsonl" && stem == id)
                    || (ext == "jsonl"
                        && stem == "transcript"
                        && path
                            .ancestors()
                            .nth(3)
                            .and_then(|p| p.file_name())
                            .is_some_and(|n| n == id))
                    || (ext == "jsonl"
                        && stem == "transcript"
                        && path
                            .parent()
                            .and_then(|p| p.file_name())
                            .is_some_and(|n| n == id))
            }
            Source::Vscode => {
                ext == "json"
                    && stem == id
                    && path.parent().is_some_and(|p| p.ends_with("chatSessions"))
            }
            Source::Grok => {
                path.file_name().is_some_and(|n| n == "chat_history.jsonl")
                    && path
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|n| n == id)
            }
            Source::Gemini => {
                // Gemini filenames may contain only the first eight ID chars;
                // always verify the complete sessionId in file metadata.
                if matches!(ext, "json" | "jsonl")
                    && stem.starts_with("session-")
                    && stem.contains(&id[..8.min(id.len())])
                {
                    let text = fs::read_to_string(&path).map_err(|e| err(&path, e))?;
                    let first = serde_json::from_str::<Value>(&text)
                        .or_else(|_| serde_json::from_str(text.lines().next().unwrap_or("")))
                        .map_err(|e| err(&path, e))?;
                    first.get("sessionId").and_then(Value::as_str) == Some(id)
                } else {
                    false
                }
            }
            _ => false,
        };
        if matched {
            found.push(path);
        }
    }
    Ok(())
}

/// OpenCode renamed its session/message tables in v2 (`session` -> `session_v2`,
/// `message` -> `session_message`). Both schemas can coexist in one file, so the
/// live schema decides which tables to read; v1 stopped being written on
/// 2026-09-09 while v2 still receives every new session.
pub(crate) fn session_table(path: &Path) -> Result<&'static str, ReadError> {
    Ok(if has_table(path, "session_v2")? {
        "session_v2"
    } else {
        "session"
    })
}

fn has_table(path: &Path, table: &str) -> Result<bool, ReadError> {
    // Table names come from this module's constants, never from caller input.
    Ok(sqlite(
        path,
        &format!(
            "SELECT json_object('n',count(*)) FROM sqlite_master WHERE type='table' AND name='{table}'"
        ),
    )?
    .first()
    .and_then(|row| row.get("n").and_then(Value::as_i64))
    .unwrap_or(0)
        > 0)
}

/// Emit the same `{id, message, parts}` envelope for both OpenCode schemas so
/// the projection layer keeps one dialect. v2 carries parts inline in
/// `session_message.data` and its own `type` column; v1 keeps them in the
/// sibling `part` table and the role inside `message.data`.
fn message_sql(path: &Path, id: &str, paging: &str) -> Result<String, ReadError> {
    // Indexed session query only: never dump the multi-GB database.
    if has_table(path, "session_v2")? {
        Ok(format!(
            "SELECT json_object('id',m.id,'message',json_object('role',m.type,'time',json_extract(m.data,'$.time'),'agent',json_extract(m.data,'$.agent'),'model',json_extract(m.data,'$.model'),'status',json_extract(m.data,'$.status'),'summary',json_extract(m.data,'$.summary')),'parts',json(COALESCE(json_extract(m.data,'$.content'),json_array(json_object('type','text','text',COALESCE(json_extract(m.data,'$.text'),json_extract(m.data,'$.summary'))))))) FROM session_message m WHERE m.session_id='{id}' ORDER BY m.seq{paging}"
        ))
    } else {
        Ok(format!(
            "SELECT json_object('id',m.id,'message',json(m.data),'parts',json((SELECT json_group_array(json(p.data)) FROM (SELECT data FROM part WHERE message_id=m.id ORDER BY time_created,id) p))) FROM message m WHERE m.session_id='{id}' ORDER BY m.time_created,m.id{paging}"
        ))
    }
}

pub fn load_database(located: &Located, id: &str, offset: usize) -> Result<Vec<Value>, ReadError> {
    let sql = message_sql(&located.path, id, &format!(" LIMIT 100 OFFSET {offset}"))?;
    sqlite(&located.path, &sql)
}

pub(crate) fn load_database_session(path: &Path, id: &str) -> Result<Vec<Value>, ReadError> {
    let path = database_path(path);
    sqlite(&path, &message_sql(&path, id, "")?)
}

pub(crate) fn database_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("opencode.db")
    } else {
        path.to_owned()
    }
}

pub(crate) fn database_child_sessions(path: &Path, id: &str) -> Result<Vec<String>, ReadError> {
    let path = database_path(path);
    sqlite(
        &path,
        &format!(
            "SELECT json_object('id',id) FROM {0} WHERE parent_id='{id}' ORDER BY id",
            session_table(&path)?
        ),
    )
    .map(|rows| {
        rows.into_iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect()
    })
}

pub fn load_document(located: &Located) -> Result<Vec<Value>, ReadError> {
    let text = fs::read_to_string(&located.path).map_err(|e| err(&located.path, e))?;
    let data: Value = serde_json::from_str(&text).map_err(|e| err(&located.path, e))?;
    match located.source {
        Source::Gemini => data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| ReadError("Gemini document missing messages array".into())),
        Source::Vscode => {
            let requests = data
                .get("requests")
                .and_then(Value::as_array)
                .ok_or_else(|| ReadError("VS Code document missing requests array".into()))?;
            Ok(requests.clone())
        }
        _ => Err(ReadError("unsupported document source".into())),
    }
}

pub(crate) fn sqlite(path: &Path, sql: &str) -> Result<Vec<Value>, ReadError> {
    // No shell interpolation; ID syntax is validated before query construction.
    // -readonly includes WAL state (unlike immutable=1) without data writes.
    // SQL emits one JSON object per row. The system CLI's -json formatter
    // stalled on real large message values; SQL JSON serialization did not.
    // Disable interactive startup customization and headers for this protocol.
    let output = Command::new("sqlite3")
        .args([
            "-init",
            "/dev/null",
            "-batch",
            "-readonly",
            "-noheader",
            "-list",
            "-cmd",
            ".timeout 2000",
        ])
        .arg("--")
        .arg(path)
        .arg(sql)
        .output()
        .map_err(|e| ReadError(format!("sqlite3 required for OpenCode: {e}")))?;
    if !output.status.success() {
        return Err(ReadError(format!(
            "sqlite3 {} ({}): {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if output.stdout.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(path, e))
}

fn err(path: &Path, e: impl std::fmt::Display) -> ReadError {
    ReadError(format!("read {}: {e}", path.display()))
}

#[cfg(test)]
mod opencode_schema_tests {
    use super::*;

    fn create(db: &Path, sql: &str) {
        let status = std::process::Command::new("sqlite3")
            .arg(db)
            .arg(sql)
            .status()
            .expect("sqlite3 available");
        assert!(status.success(), "fixture schema applies");
    }

    #[test]
    fn v2_schema_reads_inline_parts_and_ignores_non_dialogue_rows() {
        // OpenCode v2 writes session_v2/session_message with parts inside data and
        // no part table. Both schemas can coexist in one file; when the v2 tables
        // exist they are the live ones, otherwise every session created after the
        // v1 write cutoff reads as missing.
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("opencode.db");
        create(
            &db,
            "CREATE TABLE session_v2(id TEXT PRIMARY KEY,parent_id TEXT,directory TEXT,time_updated INTEGER);\
             CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,seq INTEGER,data TEXT);\
             INSERT INTO session_v2 VALUES('ses_v2only','ses_parent','/tmp/p',2);\
             INSERT INTO session_v2 VALUES('ses_parent',NULL,'/tmp/p',1);\
             INSERT INTO session_message VALUES('m1','ses_v2only','user',1,'{\"text\":\"岡山的問題\"}');\
             INSERT INTO session_message VALUES('m2','ses_v2only','assistant',2,'{\"content\":[{\"type\":\"reasoning\",\"text\":\"internal\"},{\"type\":\"text\",\"text\":\"已上架\"}]}');\
             INSERT INTO session_message VALUES('m3','ses_v2only','idle',3,'{\"outcome\":\"succeeded\"}');",
        );
        let before = fs::read(&db).unwrap();

        assert_eq!(session_table(&db).unwrap(), "session_v2");
        let page =
            crate::conversation::read(Some(&db), "opencode", "ses_v2only", "dialogue", 0, 20)
                .unwrap();
        assert_eq!(page.records.len(), 2, "idle rows are not dialogue");
        assert_eq!(page.records[0].event["role"], "user");
        assert_eq!(page.records[0].event["content"], "岡山的問題");
        assert_eq!(page.records[1].event["role"], "assistant");
        // Reasoning stays source-native and out of projected human text.
        assert_eq!(page.records[1].event["content"], "已上架");

        let events =
            crate::conversation::read(Some(&db), "opencode", "ses_v2only", "events", 0, 3).unwrap();
        assert_eq!(events.records.len(), 3);
        assert_eq!(
            events.records[1].event["parts"][0]["type"], "reasoning",
            "inline content parts stay addressable in events view"
        );

        assert_eq!(
            database_child_sessions(&db, "ses_parent").unwrap(),
            vec!["ses_v2only".to_string()]
        );
        assert_eq!(fs::read(&db).unwrap(), before, "reads must not mutate");
    }

    #[test]
    fn v1_schema_still_reads_sibling_parts_when_no_v2_table_exists() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("opencode.db");
        create(
            &db,
            "CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT,directory TEXT,time_updated INTEGER);\
             CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT);\
             CREATE TABLE part(id TEXT PRIMARY KEY,message_id TEXT,time_created INTEGER,data TEXT);\
             INSERT INTO session VALUES('ses_legacy',NULL,'/tmp/p',1);\
             INSERT INTO message VALUES('m1','ses_legacy',1,'{\"role\":\"user\"}');\
             INSERT INTO part VALUES('p1','m1',1,'{\"type\":\"text\",\"text\":\"legacy\"}');",
        );
        assert_eq!(session_table(&db).unwrap(), "session");
        let page =
            crate::conversation::read(Some(&db), "opencode", "ses_legacy", "dialogue", 0, 20)
                .unwrap();
        assert_eq!(page.records[0].event["content"], "legacy");
    }

    #[test]
    fn discovery_and_message_loading_follow_one_schema_decision() {
        // A mismatch here would report live sessions as missing: locate() must
        // query the same table that message_sql() reads. OpenCode's own
        // migration copies every v1 session into v2 (321/321 on this machine),
        // so the v2 table is the single live source once it exists.
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("opencode.db");
        create(
            &db,
            "CREATE TABLE session(id TEXT PRIMARY KEY);\
             CREATE TABLE session_v2(id TEXT PRIMARY KEY,parent_id TEXT);\
             CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,seq INTEGER,data TEXT);\
             INSERT INTO session VALUES('ses_legacy');\
             INSERT INTO session_v2 VALUES('ses_live',NULL);\
             INSERT INTO session_message VALUES('m1','ses_live','user',1,'{\"text\":\"live\"}');",
        );
        let located = locate(Some(&db), "opencode", "ses_live").unwrap();
        assert_eq!(located.path, db.canonicalize().unwrap());
        let page = crate::conversation::read(Some(&db), "opencode", "ses_live", "dialogue", 0, 20)
            .unwrap();
        assert_eq!(page.records[0].event["content"], "live");
    }
}
