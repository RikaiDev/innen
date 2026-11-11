//! Conversation compression: semantic extraction for RAG retrieval and cross-tool handoff.
//!
//! Uses conversation structure (roles, tool calls, task_state, checkpoints) as primary signals.
//! Raw transcripts can be deleted after compression.

pub mod blockers;
pub mod code_changes;
pub mod decisions;
pub mod discourse;
pub mod graph_write;
pub mod handoff;
pub mod narrative;
pub mod requirements;
pub mod scoring;

use crate::conversation::checkpoint::Checkpoint;
use crate::conversation::{ReadError, Record, Source};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Compressed conversation representation with semantic metadata for RAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedConversation {
    pub conversation_id: String,
    pub session_id: String,
    pub source: Source,
    pub project: String,
    pub last_activity: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub total_records: usize,
    pub extraction: ExtractionResult,
    pub knowledge_node_ids: Vec<String>,
    pub session_closed: bool,
    pub compressed_at: String,
}

/// Results of semantic extraction from conversation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractionResult {
    pub decisions: Vec<Decision>,
    pub code_changes: Vec<CodeChange>,
    pub blockers: Vec<Blocker>,
    pub handoff_instructions: Vec<HandoffInstruction>,
    pub task_state: Option<TaskStateSummary>,
    pub narrative_arc: Option<NarrativeArc>,
    pub unfinished_score: f32,
    pub handoff_priority: f32,
    pub last_intent: Option<String>,
    pub code_state_diff: Option<String>,
    pub user_requirements: Vec<UserRequirement>,
    pub deliverables: Vec<Deliverable>,
    #[serde(skip)]
    pub last_activity_timestamp: Option<String>,
}

/// Narrative arc for presentations/docs: Why → Scenario → Vehicle → Architecture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeArc {
    pub why: Option<String>,
    pub scenario: Option<String>,
    pub vehicle: Option<String>,
    pub architecture: Option<String>,
    pub source_ranges: NarrativeRanges,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NarrativeRanges {
    pub why: Option<(usize, usize)>,
    pub scenario: Option<(usize, usize)>,
    pub vehicle: Option<(usize, usize)>,
    pub architecture: Option<(usize, usize)>,
}

/// Decision extracted from conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub summary: String,
    pub rationale: Option<String>,
    pub source_lines: Vec<usize>,
    pub confidence: f32,
    pub category: DecisionCategory,
    pub confirmed: bool,
    pub alternatives: Vec<String>,
    pub trigger: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCategory {
    Architecture,
    Implementation,
    Tooling,
    Process,
    Design,
    Security,
    Performance,
    Format,
    Other,
}

/// Code change discussed or implemented.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChange {
    pub files: Vec<String>,
    pub change_type: CodeChangeType,
    pub summary: String,
    pub source_lines: Vec<usize>,
    pub completed: bool,
    pub related_decision: Option<String>,
    pub diff_summary: Option<String>,
    pub tool_calls: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeChangeType {
    Create,
    Modify,
    Delete,
    Refactor,
    Fix,
    Test,
    Config,
    Format,
    Security,
    Compliance,
    Other,
}

/// Blocker encountered during conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    pub description: String,
    pub category: BlockerCategory,
    pub resolved: bool,
    pub resolution: Option<String>,
    pub source_lines: Vec<usize>,
    pub severity: f32,
    pub blocking: Option<String>,
    pub failed_tool_calls: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockerCategory {
    Technical,
    Dependency,
    Permission,
    Information,
    Design,
    Resource,
    Compliance,
    Other,
}

/// Handoff instruction for session continuation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffInstruction {
    pub instruction: String,
    pub priority: f32,
    pub context: Option<String>,
    pub source_lines: Vec<usize>,
    pub instruction_type: HandoffType,
    pub explicit: bool,
    pub resolves_blocker: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HandoffType {
    ContinueTask,
    ResolveBlocker,
    VerifyChange,
    RunTests,
    UpdateDocs,
    Deploy,
    Review,
    Redesign,
    Other,
}

/// Task state summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStateSummary {
    pub objective: Option<String>,
    pub completed_work: Vec<String>,
    pub current_evidence: Vec<String>,
    pub blockers: Vec<String>,
    pub next_action: Option<String>,
    pub verification: Option<String>,
}

/// User requirement or constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRequirement {
    pub requirement: String,
    pub source_line: usize,
    pub hard_constraint: bool,
    pub category: RequirementCategory,
    pub related_deliverable: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequirementCategory {
    Format,
    Content,
    Timeline,
    Audience,
    Technical,
    Compliance,
    Other,
}

/// Deliverable requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deliverable {
    pub description: String,
    pub format: Option<String>,
    pub source_line: usize,
    pub confirmed: bool,
    pub related_requirement: Option<String>,
}

/// Compress a full conversation transcript into semantic representation.
pub fn compress_conversation(
    session_id: &str,
    source: Source,
    project: &str,
    records: &[Record],
    source_sha256: &str,
    source_bytes: u64,
    session_closed: bool,
    latest_checkpoint: Option<&Checkpoint>,
) -> Result<CompressedConversation, ReadError> {
    let conversation_id = format!("conversation:{}:{}", source.as_str(), session_id);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ReadError(e.to_string()))?
        .as_secs();
    let compressed_at = chrono::DateTime::from_timestamp(now as i64, 0)
        .ok_or_else(|| ReadError("invalid timestamp".into()))?
        .to_rfc3339();

    // Parse records into structured discourse units
    let discourse_units = discourse::parse_discourse_units(records);

    // Extract all semantic content
    let mut extraction = ExtractionResult::default();

    extraction.task_state =
        decisions::extract_task_state_from_structure(&discourse_units, latest_checkpoint);
    extraction.decisions =
        decisions::extract_decisions_from_structure(&discourse_units, &extraction.task_state);
    extraction.code_changes = code_changes::extract_code_changes_from_tool_calls(&discourse_units);
    extraction.blockers = blockers::extract_blockers_from_structure(&discourse_units);
    extraction.handoff_instructions =
        handoff::extract_handoff_from_structure(&discourse_units, latest_checkpoint);
    extraction.user_requirements =
        requirements::extract_user_requirements_from_user_messages(&discourse_units);
    extraction.deliverables = requirements::extract_deliverables_from_structure(&discourse_units);
    extraction.narrative_arc = narrative::extract_narrative_arc_from_flow(&discourse_units);

    extraction.unfinished_score = scoring::calculate_unfinished_score(&extraction);
    extraction.handoff_priority = scoring::calculate_handoff_priority(&extraction);
    extraction.last_intent = discourse::extract_last_user_intent(&discourse_units);
    extraction.code_state_diff =
        code_changes::extract_code_state_diff_from_tool_calls(&discourse_units);
    extraction.last_activity_timestamp = discourse_units.last().and_then(|u| u.timestamp.clone());

    let knowledge_node_ids = generate_knowledge_node_ids(&conversation_id, &extraction);

    let last_activity = extraction
        .last_activity_timestamp
        .clone()
        .unwrap_or_else(|| compressed_at.clone());

    Ok(CompressedConversation {
        conversation_id,
        session_id: session_id.to_string(),
        source,
        project: project.to_string(),
        last_activity,
        source_sha256: source_sha256.to_string(),
        source_bytes,
        total_records: records.len(),
        extraction,
        knowledge_node_ids,
        session_closed,
        compressed_at,
    })
}

/// Generate knowledge node IDs for extracted items.
fn generate_knowledge_node_ids(
    conversation_id: &str,
    extraction: &ExtractionResult,
) -> Vec<String> {
    let mut ids = Vec::new();
    let hash = |s: &str| -> String {
        let mut hasher = Sha256::new();
        hasher.update(s.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)[..16].to_string()
    };

    for decision in &extraction.decisions {
        ids.push(format!(
            "knowledge:decision:{}:{}",
            conversation_id,
            hash(&decision.summary)
        ));
    }
    for change in &extraction.code_changes {
        ids.push(format!(
            "knowledge:code_change:{}:{}",
            conversation_id,
            hash(&change.files.join(","))
        ));
    }
    for blocker in &extraction.blockers {
        ids.push(format!(
            "knowledge:blocker:{}:{}",
            conversation_id,
            hash(&blocker.description)
        ));
    }
    for handoff in &extraction.handoff_instructions {
        ids.push(format!(
            "knowledge:handoff:{}:{}",
            conversation_id,
            hash(&handoff.instruction)
        ));
    }
    for req in &extraction.user_requirements {
        ids.push(format!(
            "knowledge:requirement:{}:{}",
            conversation_id,
            hash(&req.requirement)
        ));
    }
    for del in &extraction.deliverables {
        ids.push(format!(
            "knowledge:deliverable:{}:{}",
            conversation_id,
            hash(&del.description)
        ));
    }
    if let Some(task_state) = &extraction.task_state {
        if let Some(obj) = &task_state.objective {
            ids.push(format!("knowledge:task:{}:{}", conversation_id, hash(obj)));
        }
    }
    if let Some(arc) = &extraction.narrative_arc {
        if let Some(why) = &arc.why {
            ids.push(format!(
                "knowledge:narrative:why:{}:{}",
                conversation_id,
                hash(why)
            ));
        }
        if let Some(arch) = &arc.architecture {
            ids.push(format!(
                "knowledge:narrative:architecture:{}:{}",
                conversation_id,
                hash(arch)
            ));
        }
    }

    ids
}

/// Public re-export for graph writing
pub use self::graph_write::write_compressed_conversation as write_to_graph;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{Record, Source};
    use serde_json::json;

    fn sample_records() -> Vec<Record> {
        vec![
            Record {
                line: 1,
                event: json!({"role": "user", "content": "We need to decide on the architecture for the new API. Why: the current system can't scale and we have compliance requirements for personal data."}),
            },
            Record {
                line: 2,
                event: json!({"role": "assistant", "content": "I recommend using a REST API with JSON over HTTP. The approach is stateless and scalable."}),
            },
            Record {
                line: 3,
                event: json!({"role": "user", "content": "Decided: we'll use REST with OpenAPI spec. Format must be pptx for stakeholders."}),
            },
            Record {
                line: 4,
                event: json!({"role": "assistant", "content": "I'll create the OpenAPI spec in api/openapi.yaml", "tool_calls": [{"name": "write", "arguments": {"path": "api/openapi.yaml", "content": "openapi: 3.0..."}}]}),
            },
            Record {
                line: 5,
                event: json!({"role": "user", "content": "The spec is done. Next we need to implement the endpoints."}),
            },
            Record {
                line: 6,
                event: json!({"role": "assistant", "content": "Creating endpoints in src/api/endpoints.rs", "tool_calls": [{"name": "write", "arguments": {"path": "src/api/endpoints.rs", "content": "// endpoints"}}]}),
            },
            Record {
                line: 7,
                event: json!({"role": "user", "content": "Blocked: the database migration hasn't been run yet. This is a compliance issue for personal data."}),
            },
            Record {
                line: 8,
                event: json!({"role": "assistant", "content": "Running migration now...", "tool_calls": [{"name": "bash", "arguments": {"command": "sqlx migrate run"}}]}),
            },
            Record {
                line: 9,
                event: json!({"role": "user", "content": "Migration complete. Verify the endpoints work. Then deploy to staging."}),
            },
            Record {
                line: 10,
                event: json!({"role": "assistant", "content": "Tests passing. Next: deploy to staging"}),
            },
        ]
    }

    #[test]
    fn test_compress_conversation() {
        let records = sample_records();
        let compressed = compress_conversation(
            "test-session-123",
            Source::Codex,
            "/test/project",
            &records,
            "abc123",
            1024,
            true,
            None,
        )
        .unwrap();

        assert_eq!(compressed.session_id, "test-session-123");
        assert!(
            compressed.extraction.unfinished_score >= 0.0
                && compressed.extraction.unfinished_score <= 1.0
        );
        assert!(
            compressed.extraction.handoff_priority >= 0.0
                && compressed.extraction.handoff_priority <= 1.0
        );
        assert!(!compressed.knowledge_node_ids.is_empty());
    }

    #[test]
    fn test_narrative_arc_extraction() {
        let records = sample_records();
        let compressed = compress_conversation(
            "test-session-123",
            Source::Codex,
            "/test/project",
            &records,
            "abc123",
            1024,
            true,
            None,
        )
        .unwrap();

        if let Some(arc) = &compressed.extraction.narrative_arc {
            assert!(arc.why.is_some() || arc.architecture.is_some());
        }
    }

    #[test]
    fn test_user_requirements() {
        let records = sample_records();
        let compressed = compress_conversation(
            "test-session-123",
            Source::Codex,
            "/test/project",
            &records,
            "abc123",
            1024,
            true,
            None,
        )
        .unwrap();

        let has_format_req = compressed
            .extraction
            .user_requirements
            .iter()
            .any(|r| r.category == RequirementCategory::Format);
        let has_compliance_req = compressed
            .extraction
            .user_requirements
            .iter()
            .any(|r| r.category == RequirementCategory::Compliance);
        assert!(has_format_req);
        assert!(has_compliance_req);
    }

    #[test]
    fn test_code_changes_from_tool_calls() {
        let records = sample_records();
        let compressed = compress_conversation(
            "test-session-123",
            Source::Codex,
            "/test/project",
            &records,
            "abc123",
            1024,
            true,
            None,
        )
        .unwrap();

        let has_create = compressed
            .extraction
            .code_changes
            .iter()
            .any(|c| c.change_type == CodeChangeType::Create);
        assert!(has_create);
        let has_tool_calls = compressed
            .extraction
            .code_changes
            .iter()
            .any(|c| !c.tool_calls.is_empty());
        assert!(has_tool_calls);
    }
}
