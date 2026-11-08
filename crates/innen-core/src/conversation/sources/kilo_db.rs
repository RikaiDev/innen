//! Kilo session storage (OpenCode-derived).
//!
//! Kilo keeps the OpenCode v1 shape (`session`/`message`/`part`) but stores
//! the main dialogue in `message`/`part` while `session_message` is present as
//! an unused table. Reading must prefer `message`/`part` when populated,
//! otherwise v1-only detection would miss Kilo sessions that carry no
//! `session_message` rows.

use super::{Located, ReadError};
use serde_json::Value;
use std::path::Path;

pub fn load_database(located: &Located, id: &str, offset: usize) -> Result<Vec<Value>, ReadError> {
    let path = database_path(&located.path);
    let sql = format!(
        "SELECT json_object('id',m.id,'message',json(m.data),'parts',json((SELECT json_group_array(json(p.data)) FROM (SELECT data FROM part WHERE message_id=m.id ORDER BY time_created,id) p))) FROM message m WHERE m.session_id='{id}' ORDER BY m.time_created,m.id LIMIT 100 OFFSET {offset}"
    );
    super::sqlite(&path, &sql)
}

pub(crate) fn load_database_session(path: &Path, id: &str) -> Result<Vec<Value>, ReadError> {
    let path = database_path(path);
    let sql = format!(
        "SELECT json_object('id',m.id,'message',json(m.data),'parts',json((SELECT json_group_array(json(p.data)) FROM (SELECT data FROM part WHERE message_id=m.id ORDER BY time_created,id) p))) FROM message m WHERE m.session_id='{id}' ORDER BY m.time_created,m.id"
    );
    super::sqlite(&path, &sql)
}

pub(crate) fn database_child_sessions(path: &Path, id: &str) -> Result<Vec<String>, ReadError> {
    let path = database_path(path);
    super::sqlite(
        &path,
        &format!("SELECT json_object('id',id) FROM session WHERE parent_id='{id}' ORDER BY id"),
    )
    .map(|rows| {
        rows.into_iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect()
    })
}

pub(crate) fn database_path(path: &Path) -> std::path::PathBuf {
    if path.is_dir() {
        path.join("kilo.db")
    } else {
        path.to_owned()
    }
}
