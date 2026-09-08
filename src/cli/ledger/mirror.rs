pub(super) fn existing_ledger_project_id(root: &std::path::Path) -> Result<Option<String>, String> {
    let manifest = root.join(".innen-ledger/manifest.json");
    if !manifest.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&manifest).map_err(|e| format!("read ledger manifest: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse ledger manifest: {e}"))?;
    value
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "ledger manifest has no project_id".to_string())
        .map(Some)
}

pub(super) fn ledger_source_kind(
    value: &str,
) -> Result<innen_core::artifact::ledger::SourceKind, String> {
    match value.to_ascii_lowercase().as_str() {
        "document" => Ok(innen_core::artifact::ledger::SourceKind::Document),
        "event" => Ok(innen_core::artifact::ledger::SourceKind::Event),
        "dataset" => Ok(innen_core::artifact::ledger::SourceKind::Dataset),
        "other" => Ok(innen_core::artifact::ledger::SourceKind::Other),
        _ => Err(format!("invalid ledger source kind: {value}")),
    }
}

pub(super) fn ledger_stage(
    value: &str,
) -> Result<innen_core::artifact::ledger::LifecycleStage, String> {
    match value.to_ascii_lowercase().as_str() {
        "intake" => Ok(innen_core::artifact::ledger::LifecycleStage::Intake),
        "authoring" => Ok(innen_core::artifact::ledger::LifecycleStage::Authoring),
        "verification" => Ok(innen_core::artifact::ledger::LifecycleStage::Verification),
        "delivery" => Ok(innen_core::artifact::ledger::LifecycleStage::Delivery),
        _ => Err(format!("invalid ledger lifecycle stage: {value}")),
    }
}

pub(super) fn mirror_ledger_receipt(
    kb_root: &std::path::Path,
    receipt: &innen_core::artifact::ledger::Receipt,
    archive_project: Option<&str>,
) -> Result<(), String> {
    let journal = innen_core::journal::Journal::open(kb_root).map_err(|e| e.to_string())?;
    // A content hash identifies bytes, while a receipt identifies this
    // immutable revision in the archive. Keep the latter in the graph so two
    // receipts with identical bytes cannot collapse into one revision node.
    let id = format!("artifact-revision:{}", receipt.id);
    let payload = serde_json::json!({
        "id": id,
        "type": "Artifact",
        "label": std::path::Path::new(&receipt.path)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("artifact"),
        "path": receipt.path,
        "sha256": receipt.sha256,
        "bytes": receipt.bytes,
        "status": format!("{:?}", receipt.stage).to_ascii_lowercase(),
        "owner_project": receipt.owner_project,
        "relation": receipt.relation,
        "authority": receipt.authority,
        "evidence_status": receipt.evidence_status,
        "provenance": receipt.provenance,
        "document_id": receipt.document_id,
        "revision_id": receipt.id,
        "approval_valid_from": receipt.approval_valid_from,
        "approval_valid_until": receipt.approval_valid_until,
        "archive_project": archive_project,
    });
    journal
        .append("node.upsert", &payload)
        .map_err(|e| format!("ledger receipt appended but KB node sync failed: {e}"))?;
    if let Some(document_id) = receipt.document_id.as_deref() {
        journal
            .append(
                "node.upsert",
                &serde_json::json!({
                    "id": document_id,
                    "type": "Document",
                    "label": document_id,
                    "archive_project": archive_project,
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| {
                format!("ledger artifact synced but document identity sync failed: {e}")
            })?;
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": document_id,
                    "type": "REVISION_OF",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger document synced but revision link failed: {e}"))?;
    }
    if receipt.relation == "belongs_to" && !receipt.owner_project.trim().is_empty() {
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": receipt.owner_project,
                    "type": "BELONGS_TO",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger node synced but KB membership sync failed: {e}"))?;
    }
    if receipt.relation == "reference" && !receipt.owner_project.trim().is_empty() {
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": receipt.owner_project,
                    "type": "REFERENCES_PROJECT",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger node synced but owner reference sync failed: {e}"))?;
    }
    if let Some(archive_project) = archive_project
        .filter(|archive| !archive.trim().is_empty())
        .filter(|archive| *archive != receipt.owner_project)
    {
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": id,
                    "to": archive_project,
                    "type": "REFERENCES_PROJECT",
                    "provenance": receipt.provenance,
                }),
            )
            .map_err(|e| format!("ledger owner synced but archive reference sync failed: {e}"))?;
    }
    Ok(())
}
pub(super) fn mirror_ledger_event(
    kb_root: &std::path::Path,
    previous: &innen_core::artifact::ledger::Receipt,
    current: &innen_core::artifact::ledger::Receipt,
    event: &innen_core::artifact::ledger::EventRecord,
    archive_project: Option<&str>,
) -> Result<(), String> {
    let journal = innen_core::journal::Journal::open(kb_root).map_err(|e| e.to_string())?;
    let revision_id = format!("artifact-revision:{}", current.id);
    let retained_archive = archive_project.map(str::to_owned).or_else(|| {
        std::fs::read_to_string(kb_root.join(".innen/journal.jsonl"))
            .ok()?
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event.get("op").and_then(|v| v.as_str()) == Some("node.upsert"))
            .filter_map(|event| event.get("payload").cloned())
            .filter(|payload| payload.get("id").and_then(|v| v.as_str()) == Some(&revision_id))
            .filter_map(|payload| {
                payload
                    .get("archive_project")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .next_back()
    });
    let event_id = format!(
        "artifact-ledger-event:{}:{}",
        event.receipt_id, event.sequence
    );
    journal
        .append(
            "node.upsert",
            &serde_json::json!({
                "id": event_id,
                "type": "ArtifactLedgerEvent",
                "label": format!("{} {}", event.kind, event.receipt_id),
                "receipt_id": event.receipt_id,
                "event_date": event.event_date,
                "new_owner_project": event.new_owner_project,
                "new_relation": event.new_relation,
                "provenance": event.provenance,
            }),
        )
        .map_err(|e| format!("ledger event appended but KB event sync failed: {e}"))?;
    if event.kind == "reclassify" {
        let old_edge = if previous.relation == "belongs_to" {
            "BELONGS_TO"
        } else {
            "REFERENCES_PROJECT"
        };
        if !previous.owner_project.trim().is_empty() {
            journal
                .append(
                    "edge.retract",
                    &serde_json::json!({
                        "from": format!("artifact-revision:{}", previous.id),
                        "to": previous.owner_project,
                        "type": old_edge,
                    }),
                )
                .map_err(|e| format!("ledger event synced but old edge retract failed: {e}"))?;
        }
        mirror_ledger_receipt(kb_root, current, retained_archive.as_deref())?;
    } else if event.kind == "relocate" {
        mirror_ledger_receipt(kb_root, current, retained_archive.as_deref())?;
    }
    if event.kind == "supersede" {
        let old_id = event
            .supersedes
            .as_deref()
            .ok_or_else(|| "supersede event has no superseded receipt".to_string())?;
        let journal = innen_core::journal::Journal::open(kb_root).map_err(|e| e.to_string())?;
        journal
            .append(
                "node.upsert",
                &serde_json::json!({
                    "id": format!("artifact-revision:{}", old_id),
                    "status": "superseded",
                    "superseded_by": format!("artifact-revision:{}", current.id),
                    "provenance": event.provenance,
                }),
            )
            .map_err(|e| format!("ledger event synced but superseded node update failed: {e}"))?;
        journal
            .append(
                "edge.assert",
                &serde_json::json!({
                    "from": format!("artifact-revision:{}", current.id),
                    "to": format!("artifact-revision:{}", old_id),
                    "type": "SUPERSEDES",
                    "provenance": event.provenance,
                }),
            )
            .map_err(|e| format!("ledger event synced but supersession link failed: {e}"))?;
    }
    Ok(())
}

// P2 `artifact add --file <path> [--project <id>]`.
