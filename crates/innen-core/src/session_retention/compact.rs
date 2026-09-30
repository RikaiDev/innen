//! Compaction: the zero-token retention output for a closed native session.
//!
//! Keeps only the user/assistant dialogue (with original source line numbers
//! for evidence expansion) and drops tool output, file dumps and attachments,
//! which are almost all of a native session's bytes and are never needed to
//! continue work. The compact transcript becomes the knowledge node that
//! authorizes purging the native bundle.

use super::*;
use crate::conversation::sources::Located;
use std::io::Write;

/// Directory, relative to the knowledge-base root, holding compact transcripts.
/// It lives under the gitignored harvest inbox: private transcripts never enter Git.
pub const COMPACT_DIR: &str = "00-inbox/harvest/compact";

#[derive(Debug, Clone, Serialize)]
pub struct CompactRecord {
    pub node_id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub messages: usize,
    pub source_sha256: String,
    pub source_bytes: u64,
}

fn dialogue_lines(line: usize, event: &Value, out: &mut Vec<Value>) {
    // VS Code projects one request as a pair of messages; every other source is one message.
    if let Some(messages) = event.get("messages").and_then(Value::as_array) {
        for message in messages {
            dialogue_lines(line, message, out);
        }
        return;
    }
    let role = event.get("role").and_then(Value::as_str).unwrap_or("");
    let content = event.get("content").and_then(Value::as_str).unwrap_or("");
    if matches!(role, "user" | "assistant") && !content.trim().is_empty() {
        let mut row = json!({"line": line, "role": role, "content": content});
        if let Some(timestamp) = event.get("timestamp").filter(|t| !t.is_null()) {
            row["timestamp"] = timestamp.clone();
        }
        out.push(row);
    }
}

/// Write the compact transcript and record conversation, compact-transcript,
/// provenance edge and retention proof in the journal. Refuses when the native
/// session is active or its bytes change while compacting.
pub(super) fn compact_and_attest(
    root: &Path,
    candidate: &Candidate,
) -> Result<CompactRecord, Error> {
    let before = bundle(candidate)?;
    match session_activity(&before.targets) {
        Activity::Idle => {}
        Activity::Active => {
            return Err(Error::Proof(
                "native session has an open file handle".into(),
            ))
        }
        Activity::Unknown(detail) => {
            return Err(Error::Proof(format!(
                "native session activity could not be verified: {detail}"
            )))
        }
    }
    let source = candidate.source.as_str();
    let dir = root.join(COMPACT_DIR).join(source);
    fs::create_dir_all(&dir).map_err(|error| io(&dir, error))?;
    let path = dir.join(format!("{}.jsonl", candidate.id));
    let partial = dir.join(format!("{}.jsonl.partial", candidate.id));
    let located = Located {
        source: candidate.source,
        path: candidate.path.clone(),
    };

    let result = (|| -> Result<(u64, usize, String), Error> {
        let mut file = File::create(&partial).map_err(|error| io(&partial, error))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        let mut messages = 0usize;
        let mut rows = Vec::new();
        crate::conversation::stream_dialogue(&located, &candidate.id, |line, event| {
            rows.clear();
            dialogue_lines(line, &event, &mut rows);
            for row in &rows {
                let mut encoded = serde_json::to_vec(row).expect("dialogue row serializes");
                encoded.push(b'\n');
                file.write_all(&encoded)
                    .map_err(|error| crate::conversation::ReadError(error.to_string()))?;
                hasher.update(&encoded);
                bytes += encoded.len() as u64;
                messages += 1;
            }
            Ok(())
        })
        .map_err(|error| Error::Conversation(error.to_string()))?;
        file.sync_all().map_err(|error| io(&partial, error))?;
        Ok((bytes, messages, hex(&hasher.finalize())))
    })();
    let (bytes, messages, sha256) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    };
    let after = bundle(candidate)?;
    if after.sha256 != before.sha256 {
        let _ = fs::remove_file(&partial);
        return Err(Error::Proof(
            "native session changed while compacting".into(),
        ));
    }
    fs::rename(&partial, &path).map_err(|error| io(&path, error))?;

    let conversation_id = format!("conversation:{source}:{}", candidate.id);
    let node_id = format!(
        "compact-transcript:{source}:{}:{}",
        candidate.id,
        &sha256[..16]
    );
    let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
    let journal = Journal::open(root).map_err(|error| Error::Journal(error.to_string()))?;
    let append = |op: &str, payload: Value| {
        journal
            .append(op, &payload)
            .map_err(|error| Error::Journal(error.to_string()))
    };
    let graph = load_graph(root)?;
    if !graph.nodes.contains_key(&conversation_id) {
        append(
            "node.upsert",
            json!({
                "id": conversation_id, "type": "Conversation",
                "label": format!("{source} session {}", candidate.id),
                "provenance": "innen retention sweep",
                "project": candidate.project, "modified": candidate.modified,
            }),
        )?;
    }
    append(
        "node.upsert",
        json!({
            "id": node_id, "type": "CompactTranscript",
            "label": format!("compact transcript {source}:{}", candidate.id),
            "provenance": "innen retention sweep",
            "path": relative, "sha256": sha256, "bytes": bytes, "messages": messages,
            "source_sha256": before.sha256, "source_bytes": before.bytes,
            "observed_utc": observed_utc_now(),
        }),
    )?;
    append(
        "edge.assert",
        json!({"from": node_id, "type": "DERIVED_FROM", "to": conversation_id}),
    )?;
    append(
        "node.upsert",
        json!({
            "id": format!("session-retention:{source}:{}:{}", candidate.id, &before.sha256[..16]),
            "type": "SessionRetentionReceipt",
            "label": format!("compact retention proof {source}:{}", candidate.id),
            "provenance": "innen retention sweep (compact)",
            "schema": SCHEMA,
            "session_id": candidate.id, "source": source,
            "source_sha256": before.sha256, "source_bytes": before.bytes,
            "knowledge_node_ids": [node_id],
            "extraction_complete": true, "session_closed": true,
            "observed_utc": observed_utc_now(),
        }),
    )?;
    Ok(CompactRecord {
        node_id,
        path: relative,
        sha256,
        bytes,
        messages,
        source_sha256: before.sha256,
        source_bytes: before.bytes,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
