use super::activity::Activity;
use super::*;
use crate::session_retention::types::BLOCKER_NEEDS_COMPRESSION;

pub(super) fn assess_candidate(
    root: &Path,
    candidate: &Candidate,
    retention_days: u64,
) -> Result<Assessment, Error> {
    let graph = load_graph(root)?;
    assess_candidate_with_graph(&graph, candidate, retention_days)
}

pub(super) fn assess_candidate_with_graph(
    graph: &crate::graph::Materialized,
    candidate: &Candidate,
    retention_days: u64,
) -> Result<Assessment, Error> {
    let cutoff = cutoff_utc(retention_days);
    let targets = candidate_targets(candidate);
    if !targets.iter().any(|target| target.exists()) {
        return Ok(Assessment {
            session_id: candidate.id.clone(),
            source: candidate.source.as_str().into(),
            modified: candidate.modified.clone(),
            cutoff_utc: cutoff,
            source_bytes: 0,
            source_sha256: String::new(),
            eligible: false,
            blockers: vec!["native bundle already removed".into()],
            blocker_codes: vec![BLOCKER_ALREADY_CLEANED.into()],
            targets,
        });
    }
    if !is_trustworthy_modified(candidate.modified.as_deref())
        || candidate
            .modified
            .as_deref()
            .is_some_and(|modified| modified >= cutoff.as_str())
    {
        let (message, code) = if !is_trustworthy_modified(candidate.modified.as_deref()) {
            (
                "missing trustworthy modified time",
                BLOCKER_MODIFIED_TIME_UNKNOWN,
            )
        } else {
            ("inside retention window", BLOCKER_RETENTION_WINDOW)
        };
        return Ok(Assessment {
            session_id: candidate.id.clone(),
            source: candidate.source.as_str().into(),
            modified: candidate.modified.clone(),
            cutoff_utc: cutoff,
            source_bytes: measured_bytes(candidate),
            source_sha256: String::new(),
            eligible: false,
            blockers: vec![message.into()],
            blocker_codes: vec![code.into()],
            targets: candidate_targets(candidate),
        });
    }

    let bundle = bundle(candidate)?;
    let mut blockers = Vec::new();
    let mut blocker_codes = Vec::new();

    if matches!(candidate.source, Source::Opencode | Source::Kilo) {
        let children = match candidate.source {
            Source::Kilo => {
                sources::kilo_db::database_child_sessions(&candidate.path, &candidate.id)
            }
            _ => sources::database_child_sessions(&candidate.path, &candidate.id),
        }
        .map_err(|error| Error::Conversation(error.to_string()))?;
        if !children.is_empty() {
            blockers.push(format!(
                "{} delete would also remove child sessions; attest and purge children first: {}",
                candidate.source.as_str(),
                children.join(",")
            ));
            blocker_codes.push(BLOCKER_CHILD_SESSIONS.into());
        }
    }

    let proof_candidates = graph
        .nodes
        .values()
        .filter(|node| {
            node.get("schema").and_then(Value::as_str) == Some(SCHEMA)
                && node.get("session_id").and_then(Value::as_str) == Some(candidate.id.as_str())
                && node.get("source").and_then(Value::as_str) == Some(candidate.source.as_str())
                && node.get("extraction_complete").and_then(Value::as_bool) == Some(true)
                && node.get("session_closed").and_then(Value::as_bool) == Some(true)
        })
        .collect::<Vec<_>>();

    let already_compressed = !proof_candidates.is_empty();
    if already_compressed {
        let proof = proof_candidates.into_iter().find(|node| {
            node.get("source_sha256").and_then(Value::as_str) == Some(bundle.sha256.as_str())
        });
        match proof {
            None => {
                blockers.push(
                    "no exact, complete extraction attestation for current source bytes".into(),
                );
                blocker_codes.push(BLOCKER_ATTESTATION_MISSING.into());
            }
            Some(proof) => {
                let conversation_id = format!(
                    "conversation:{}:{}",
                    candidate.source.as_str(),
                    candidate.id
                );
                let knowledge = proof
                    .get("knowledge_node_ids")
                    .and_then(Value::as_array)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if knowledge.is_empty() {
                    blockers.push("attestation has no knowledge nodes".into());
                    blocker_codes.push(BLOCKER_KNOWLEDGE_MISSING.into());
                }
                for id in &knowledge {
                    if !graph.nodes.contains_key(id) {
                        blockers.push(format!("knowledge node disappeared: {id}"));
                        blocker_codes.push(BLOCKER_KNOWLEDGE_MISSING.into());
                        continue;
                    }
                    let linked = graph.edges.iter().any(|edge| {
                        !edge.retracted
                            && edge.from == *id
                            && edge.to == conversation_id
                            && matches!(
                                edge.edge.to_string().as_str(),
                                "DERIVED_FROM" | "DISCUSSED_IN"
                            )
                    });
                    if !linked {
                        blockers.push(format!("knowledge provenance edge disappeared: {id}"));
                        blocker_codes.push(BLOCKER_KNOWLEDGE_MISSING.into());
                    }
                }
            }
        }
    } else {
        blockers.push("session needs compression before purge eligibility".into());
        blocker_codes.push(BLOCKER_NEEDS_COMPRESSION.into());
    }

    if blockers.is_empty() {
        match session_activity(&liveness_targets(candidate.source, &bundle.targets)) {
            Activity::Idle => {}
            Activity::Active => {
                blockers.push("native session has an open file handle".into());
                blocker_codes.push(BLOCKER_SESSION_ACTIVE.into());
            }
            Activity::Unknown(detail) => {
                blockers.push(format!(
                    "native session activity could not be verified: {detail}"
                ));
                blocker_codes.push(BLOCKER_ACTIVITY_UNKNOWN.into());
            }
        }
    }

    Ok(Assessment {
        session_id: candidate.id.clone(),
        source: candidate.source.as_str().into(),
        modified: candidate.modified.clone(),
        cutoff_utc: cutoff,
        source_bytes: bundle.bytes,
        source_sha256: bundle.sha256,
        eligible: blockers.is_empty(),
        blockers,
        blocker_codes: blocker_codes
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        targets: bundle.targets,
    })
}

pub(super) fn is_trustworthy_modified(modified: Option<&str>) -> bool {
    modified
        .and_then(|value| value.get(..4))
        .and_then(|year| year.parse::<u16>().ok())
        .is_some_and(|year| year >= 1970)
}
