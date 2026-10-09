//! The retrieval entry point: journal replay, an in-RAM search index, candidate
//! ranking, baseline/authority resolution, and the JSON payload both views
//! render from.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value as _;
use tantivy::{doc, Index, TantivyDocument};

use crate::graph::materialize;
use crate::index::{ensure_tokenizer, schema};
use crate::journal::{observed_utc_now, Journal};

use super::admission::{has_all_search_terms, has_entity_anchor, literal_candidate_scores};
use super::node_identity::{node_kind, node_searchable_text, node_text};
use super::query_terms::{
    extract_constraint_cues, extract_operation_cue, extract_query_terms, has_asset_edit_intent,
    sanitize_for_tantivy,
};
use super::traversal::traverse_candidates;
use super::types::TaskEntryOptions;

/// Main entry point for task-context retrieval.
pub fn task_entry(root: &Path, options: &TaskEntryOptions) -> Result<Value, String> {
    // 1. Check journal presence without creating cwd/.innen
    let journal_file = root.join(".innen").join("journal.jsonl");
    if !journal_file.is_file() {
        return Err("knowledge journal unavailable; supply --root <knowledge-base>".into());
    }

    let journal = Journal::open(root).map_err(|e| e.to_string())?;
    let entries = journal.read_all().map_err(|e| e.to_string())?;
    let events: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "op": e.op,
                "payload": e.payload,
                "observed_utc": e.observed_utc,
            })
        })
        .collect();

    let as_of = options.as_of.clone().unwrap_or_else(observed_utc_now);
    let materialized = materialize(&events, Some(&as_of), options.include_expired);

    // 2. Build in-RAM Tantivy index with CJK tokenizer
    let index_schema = schema();
    let body_field = index_schema.get_field("body").expect("schema defines body");
    let id_field = index_schema.get_field("id").expect("schema defines id");
    let index = Index::create_in_ram(index_schema);
    ensure_tokenizer(&index);
    {
        let mut writer = index.writer(15_000_000).map_err(|e| e.to_string())?;
        let mut ids: Vec<&String> = materialized.nodes.keys().collect();
        ids.sort();
        for id in ids {
            let node = &materialized.nodes[id];
            let text = node_searchable_text(node);
            writer
                .add_document(doc!(
                    body_field => text,
                    id_field => id.clone(),
                ))
                .map_err(|e| e.to_string())?;
        }
        writer.commit().map_err(|e| e.to_string())?;
    }

    let reader = index.reader().map_err(|e| e.to_string())?;
    let searcher = reader.searcher();

    // 3. Search seeds: Tantivy FTS + literal substring
    let terms = extract_query_terms(&options.q);
    let identity_terms: Vec<&str> = terms
        .iter()
        .map(String::as_str)
        .filter(|term| {
            let ascii_word = term.chars().all(|c| c.is_ascii_alphanumeric());
            let chars = term.chars().count();
            (ascii_word && chars >= 3) || (!ascii_word && chars >= 2)
        })
        .collect();
    // ASCII words of at least three characters are useful entity anchors
    // (e.g. `weemed`). When one is present, generic intent terms such as
    // `標準字` must never admit a different named entity.
    let entity_terms: Vec<&str> = identity_terms
        .iter()
        .copied()
        .filter(|term| term.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect();
    let mut fts_scores: BTreeMap<String, f64> = BTreeMap::new();

    if !options.q.trim().is_empty() {
        let parser = QueryParser::for_index(&index, vec![body_field]);
        let sanitized = sanitize_for_tantivy(&options.q);
        let query_text = if !terms.is_empty() {
            terms.join(" ")
        } else {
            sanitized
        };

        if let Ok(parsed) = parser.parse_query(&query_text) {
            if let Ok(top) = searcher.search(&parsed, &TopDocs::with_limit(100).order_by_score()) {
                let mut scored: Vec<(String, f32)> = Vec::new();
                for (bm25, addr) in top {
                    if let Ok(doc) = searcher.doc::<TantivyDocument>(addr) {
                        if let Some(id) = doc.get_first(id_field).and_then(|v| v.as_str()) {
                            scored.push((id.to_string(), bm25));
                        }
                    }
                }
                let max = scored.iter().map(|(_, s)| *s).fold(0.0_f32, f32::max);
                for (id, bm25) in scored {
                    let norm = if max == 0.0 {
                        0.0
                    } else {
                        (f64::from(bm25) / f64::from(max)).clamp(0.0, 1.0)
                    };
                    fts_scores.insert(id, norm);
                }
            }
        }
    }

    // 4. Literal substring matching on identity fields. FTS is broad by
    // design: a single generic identity term is noise, while a conjunctive
    // match for the complete request is a useful topic candidate.
    let q_lower = options.q.trim().to_lowercase();
    let allow_partial_identity = has_asset_edit_intent(&options.q);
    let mut candidate_scores: BTreeMap<String, (f64, String)> = BTreeMap::new(); // id -> (score, why)

    for (id, fts_score) in &fts_scores {
        let Some(node) = materialized.nodes.get(id) else {
            continue;
        };
        let admitted = if entity_terms.is_empty() {
            has_all_search_terms(node, &identity_terms)
        } else {
            has_entity_anchor(node, &entity_terms) || has_all_search_terms(node, &identity_terms)
        };
        if admitted {
            candidate_scores.insert(id.clone(), (*fts_score, "fts_identity_match".to_string()));
        }
    }

    literal_candidate_scores(
        &materialized.nodes,
        &terms,
        &identity_terms,
        &entity_terms,
        &q_lower,
        allow_partial_identity,
        &mut candidate_scores,
    );

    // 5. Rank candidate seeds (preserving BM25 / literal relevance)
    let mut ranked_candidates: Vec<(String, f64, String)> = candidate_scores
        .into_iter()
        .map(|(id, (score, why))| (id, score, why))
        .collect();
    ranked_candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // 6. Bounded, typed relationship traversal from candidates
    let (linked_projects, connected_decisions, candidate_status_map, approved_candidates) =
        traverse_candidates(&ranked_candidates, &materialized.nodes, &materialized.edges);

    // 7. Baseline & Authority Analysis (Intent-specific)
    let is_asset_edit = has_asset_edit_intent(&options.q);
    let mut missing_evidence: Vec<String> = Vec::new();

    let baseline = if approved_candidates.len() == 1 {
        let (id, label, status) = &approved_candidates[0];
        Some(json!({
            "id": id,
            "label": label,
            "status": status,
        }))
    } else {
        None
    };

    let resolution: &'static str = if ranked_candidates.is_empty() {
        missing_evidence
            .push("No matching entities or assets found in knowledge base for query".to_string());
        "missing"
    } else if is_asset_edit {
        // Asset-edit intent: require editable vector baseline and authority
        if approved_candidates.len() > 1 {
            missing_evidence.push(
                "Multiple approved baseline candidates found; selection is ambiguous".to_string(),
            );
            "ambiguous"
        } else if baseline.is_none() {
            missing_evidence
                .push("No approved editable baseline found for candidate asset(s)".to_string());
            missing_evidence
                .push("No vector master or approval source recorded in knowledge base".to_string());
            if linked_projects.is_empty() {
                missing_evidence.push("No authoritative project or brand ownership link recorded for candidate asset(s)".to_string());
            }
            "missing"
        } else if linked_projects.len() > 1 {
            "ambiguous"
        } else {
            "ready"
        }
    } else {
        // General knowledge query: retrieved context is useful, but it does
        // not establish task completeness or authorization.
        if linked_projects.len() > 1 {
            "ambiguous"
        } else {
            "retrieved"
        }
    };

    let operation = extract_operation_cue(&options.q);
    let constraints_cues = extract_constraint_cues(&options.q);

    let total = ranked_candidates.len();
    let selected: Vec<(String, f64, String)> = ranked_candidates
        .into_iter()
        .skip(options.offset)
        .take(options.limit)
        .collect();

    let next = options.offset.saturating_add(selected.len());
    let next_offset = if next < total { Some(next) } else { None };

    let is_evidence_view = options.view == "evidence";

    // Structured argv for evidence expansion
    let mut argv = vec![
        "innen".to_string(),
        "query".to_string(),
        "--root".to_string(),
        root.to_string_lossy().into_owned(),
        "--q".to_string(),
        options.q.clone(),
        "--view".to_string(),
        "evidence".to_string(),
    ];
    if let Some(as_of_val) = &options.as_of {
        argv.push("--as-of".to_string());
        argv.push(as_of_val.clone());
    }
    if options.limit != 20 {
        argv.push("--limit".to_string());
        argv.push(options.limit.to_string());
    }
    if options.offset != 0 {
        argv.push("--offset".to_string());
        argv.push(options.offset.to_string());
    }
    if options.include_expired {
        argv.push("--include-expired".to_string());
    }

    let mut out = json!({
        "scope": "task_context_entry",
        "resolution": resolution,
        "user_request": options.q,
        "operation_cue": operation,
        "constraints_cue": {
            "extracted": constraints_cues,
            "authoritative_request": options.q,
            "note": "Extracted cues are heuristic hypotheses only; original user_request is authoritative",
        },
        "baseline": baseline,
        "linked_projects": linked_projects.into_iter().collect::<Vec<_>>(),
        "decisions": connected_decisions,
        "missing_evidence": missing_evidence,
        "total": total,
        "next_offset": next_offset,
        "evidence_expansion": {
            "argv": argv,
            "hint": "Run with --view evidence for complete node payloads and physical journal event history",
        },
    });

    if is_evidence_view {
        let items: Vec<Value> = selected
            .into_iter()
            .map(|(id, score, why)| {
                let node = &materialized.nodes[&id];
                let label = node_text(node, "label");
                let kind = node_kind(node);
                let status = candidate_status_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let history: Vec<Value> = entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        (e.op == "node.upsert" && node_text(&e.payload, "id") == id)
                            || (matches!(e.op.as_str(), "edge.assert" | "edge.retract")
                                && (node_text(&e.payload, "from") == id
                                    || node_text(&e.payload, "to") == id))
                    })
                    .map(|(i, e)| {
                        json!({
                            "line": i + 1,
                            "event": e.id,
                            "at": e.observed_utc,
                            "op": e.op,
                            "payload": e.payload,
                        })
                    })
                    .collect();

                json!({
                    "id": id,
                    "kind": kind,
                    "label": label,
                    "score": score,
                    "status": status,
                    "why": why,
                    "node": node,
                    "node_sha256": crate::ids::sha256_hex(node.to_string().as_bytes()),
                    "history": history,
                })
            })
            .collect();
        out["candidates"] = Value::Array(items);
    } else {
        out["fields"] = json!(["id", "kind", "label", "score", "status", "why"]);
        let rows: Vec<Value> = selected
            .into_iter()
            .map(|(id, score, why)| {
                let node = &materialized.nodes[&id];
                let label = node_text(node, "label");
                let kind = node_kind(node);
                let status = candidate_status_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                json!([id, kind, label, score, status, why])
            })
            .collect();
        out["rows"] = Value::Array(rows);
    }

    Ok(out)
}
