//! Write compressed conversation to graph.

use super::{CompressedConversation, ExtractionResult};
use crate::graph::Materialized;
use serde_json::{json, Value};

/// Write compressed conversation to graph as nodes and edges.
pub fn write_compressed_conversation(
    graph: &mut Materialized,
    compressed: &CompressedConversation,
) -> Result<(), crate::conversation::ReadError> {
    let conversation_id = &compressed.conversation_id;

    let conv_node = json!({
        "id": conversation_id, "type": "Conversation",
        "label": format!("conversation {}:{}", compressed.source.as_str(), compressed.session_id),
        "source": compressed.source.as_str(), "session_id": compressed.session_id,
        "project": compressed.project, "source_sha256": compressed.source_sha256,
        "source_bytes": compressed.source_bytes, "total_records": compressed.total_records,
        "session_closed": compressed.session_closed, "compressed_at": compressed.compressed_at,
        "unfinished_score": compressed.extraction.unfinished_score,
        "handoff_priority": compressed.extraction.handoff_priority,
        "last_intent": compressed.extraction.last_intent,
        "code_state_diff": compressed.extraction.code_state_diff,
        "blockers": compressed.extraction.blockers.iter().map(|b| b.description.clone()).collect::<Vec<_>>(),
        "knowledge_node_ids": compressed.knowledge_node_ids,
        "narrative_arc": serde_json::to_value(&compressed.extraction.narrative_arc).unwrap_or(json!(null)),
        "user_requirements": compressed.extraction.user_requirements.iter().map(|r| r.requirement.clone()).collect::<Vec<_>>(),
        "deliverables": compressed.extraction.deliverables.iter().map(|d| d.description.clone()).collect::<Vec<_>>(),
        "provenance": "innen compression", "schema": "innen.conversation.v1",
    });
    graph.upsert_node(conv_node);

    for (idx, knowledge_id) in compressed.knowledge_node_ids.iter().enumerate() {
        let knowledge_node = json!({
            "id": knowledge_id, "type": "Knowledge",
            "label": format!("knowledge from {}", conversation_id),
            "source_conversation": conversation_id, "extraction_index": idx,
            "provenance": "innen compression", "schema": "innen.knowledge.v1",
        });
        graph.upsert_node(knowledge_node);

        let edge_type = determine_edge_type(idx, &compressed.extraction);
        let edge = json!({
            "from": knowledge_id, "to": conversation_id, "edge": edge_type,
            "provenance": "innen compression",
        });
        graph.upsert_edge(edge);
    }

    Ok(())
}

fn determine_edge_type(idx: usize, extraction: &ExtractionResult) -> &'static str {
    let mut offset = 0;
    if idx < extraction.decisions.len() {
        return "DERIVED_FROM";
    }
    offset += extraction.decisions.len();
    if idx < offset + extraction.code_changes.len() {
        return "DERIVED_FROM";
    }
    offset += extraction.code_changes.len();
    if idx < offset + extraction.blockers.len() {
        return "DISCUSSED_IN";
    }
    offset += extraction.blockers.len();
    if idx < offset + extraction.handoff_instructions.len() {
        return "DISCUSSED_IN";
    }
    offset += extraction.handoff_instructions.len();
    if idx < offset + extraction.user_requirements.len() {
        return "DISCUSSED_IN";
    }
    offset += extraction.user_requirements.len();
    if idx < offset + extraction.deliverables.len() {
        return "DISCUSSED_IN";
    }
    "DISCUSSED_IN"
}

// Extension trait for Materialized graph
trait GraphExt {
    fn upsert_node(&mut self, node: Value);
    fn upsert_edge(&mut self, edge: Value);
}

impl GraphExt for Materialized {
    fn upsert_node(&mut self, node: Value) {
        if let Some(id) = node.get("id").and_then(|v| v.as_str()) {
            self.nodes.insert(id.to_string(), node);
        }
    }

    fn upsert_edge(&mut self, edge: Value) {
        let from = edge.get("from").and_then(|v| v.as_str()).unwrap_or("");
        let to = edge.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let edge_type = edge.get("edge").and_then(|v| v.as_str()).unwrap_or("");
        self.edges.retain(|e| {
            !(e.from == from && e.to == to && e.edge.to_string() == format!("\"{}\"", edge_type))
        });
    }
}
