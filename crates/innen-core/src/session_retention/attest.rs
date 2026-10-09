//! Writing a retention proof: bind a closed native session's current bytes to
//! the knowledge nodes already linked to its canonical conversation node.

use super::*;

pub fn attest(
    root: &Path,
    source_name: &str,
    session_id: &str,
    source_root: Option<&Path>,
    knowledge_node_ids: &[String],
    session_closed: bool,
) -> Result<Attestation, Error> {
    if !session_closed {
        return Err(Error::Proof(
            "--session-closed is required; active or ambiguous sessions are never attestable"
                .into(),
        ));
    }
    if knowledge_node_ids.is_empty() {
        return Err(Error::Proof(
            "at least one --knowledge-node is required".into(),
        ));
    }
    let source = Source::parse(source_name).map_err(|error| Error::Proof(error.to_string()))?;
    let candidate = find_candidate(source, session_id, source_root)?;
    let bundle = bundle(&candidate)?;
    match session_activity(&bundle.targets) {
        Activity::Idle => {}
        Activity::Active => {
            return Err(Error::Proof(
                "native session has an open file handle; close the coding tool before attesting"
                    .into(),
            ))
        }
        Activity::Unknown(detail) => {
            return Err(Error::Proof(format!(
                "native session activity could not be verified: {detail}"
            )))
        }
    }
    let graph = load_graph(root)?;
    let conversation_id = format!("conversation:{}:{}", source.as_str(), session_id);
    if !graph.nodes.contains_key(&conversation_id) {
        return Err(Error::Proof(format!(
            "missing canonical conversation node {conversation_id}"
        )));
    }
    let knowledge: BTreeSet<_> = knowledge_node_ids.iter().cloned().collect();
    for id in &knowledge {
        if !graph.nodes.contains_key(id) {
            return Err(Error::Proof(format!("missing knowledge node {id}")));
        }
    }
    let linked: BTreeSet<_> = graph
        .edges
        .iter()
        .filter(|edge| {
            !edge.retracted
                && edge.to == conversation_id
                && matches!(
                    edge.edge.to_string().as_str(),
                    "DERIVED_FROM" | "DISCUSSED_IN"
                )
        })
        .map(|edge| edge.from.clone())
        .collect();
    if !knowledge.iter().all(|id| linked.contains(id)) {
        return Err(Error::Proof(
            "every knowledge node must have an active DERIVED_FROM or DISCUSSED_IN edge to the conversation"
                .into(),
        ));
    }

    let id = format!(
        "session-retention:{}:{}:{}",
        source.as_str(),
        session_id,
        &bundle.sha256[..16]
    );
    let payload = json!({
        "id": id,
        "type": "SessionRetentionReceipt",
        "label": format!("verified retention proof {}:{}", source.as_str(), session_id),
        "provenance": "innen retention attest",
        "schema": SCHEMA,
        "session_id": session_id,
        "source": source.as_str(),
        "source_sha256": bundle.sha256,
        "source_bytes": bundle.bytes,
        "knowledge_node_ids": knowledge_node_ids,
        "extraction_complete": true,
        "session_closed": true,
        "observed_utc": observed_utc_now(),
    });
    Journal::open(root)
        .map_err(|error| Error::Journal(error.to_string()))?
        .append("node.upsert", &payload)
        .map_err(|error| Error::Journal(error.to_string()))?;
    Ok(Attestation {
        id: payload["id"].as_str().unwrap_or_default().to_owned(),
        session_id: session_id.to_owned(),
        source: source.as_str().to_owned(),
        source_sha256: payload["source_sha256"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        knowledge_node_ids: knowledge_node_ids.to_vec(),
        session_closed: true,
    })
}
