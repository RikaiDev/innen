//! Task-context entry: evidence-backed resolution of intent, assets, projects,
//! and baselines without requiring project IDs or falling back to cwd.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value as _;
use tantivy::{doc, Index, TantivyDocument};

use crate::graph::{materialize, EdgeType, NodeType};
use crate::index::{ensure_tokenizer, schema};
use crate::journal::{observed_utc_now, Journal};

/// Options for task-context retrieval.
#[derive(Debug, Clone)]
pub struct TaskEntryOptions {
    pub q: String,
    pub as_of: Option<String>,
    pub limit: usize,
    pub offset: usize,
    pub include_expired: bool,
    pub view: String, // "context" (default compact brief) or "evidence"
}

impl Default for TaskEntryOptions {
    fn default() -> Self {
        Self {
            q: String::new(),
            as_of: None,
            limit: 20,
            offset: 0,
            include_expired: false,
            view: "context".to_string(),
        }
    }
}

/// Helper to extract text from a node.
fn node_text<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Extract display kind from node.
fn node_kind(node: &Value) -> String {
    match node.get("type").and_then(|v| v.as_str()) {
        Some(s) => s.parse::<NodeType>().unwrap().to_string(),
        None => "Custom".to_string(),
    }
}

/// Helper to concatenate all relevant metadata (label, aliases, summary, body, path)
/// for indexing into Tantivy.
pub fn node_searchable_text(node: &Value) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let label = node_text(node, "label");
    if !label.is_empty() {
        parts.push(label);
    }
    let name = node_text(node, "name");
    if !name.is_empty() {
        parts.push(name);
    }
    let body = node_text(node, "body");
    if !body.is_empty() {
        parts.push(body);
    }
    if let Some(s) = node.get("summary").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    if let Some(arr) = node.get("summary").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    parts.push(s);
                }
            }
        }
    }
    if let Some(s) = node.get("aliases").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    if let Some(arr) = node.get("aliases").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    parts.push(s);
                }
            }
        }
    }
    if let Some(s) = node.get("alias").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    let path = node_text(node, "path");
    if !path.is_empty() {
        parts.push(path);
    }
    for key in ["document_id", "revision_id"] {
        let value = node_text(node, key);
        if !value.is_empty() {
            parts.push(value);
        }
    }
    parts.join(" ")
}

/// Identity-bearing fields used to admit a task candidate. Body text is kept
/// for evidence search, but a generic body term must not make an unrelated
/// node a candidate for a named task.
fn node_identity_text(node: &Value) -> String {
    let mut parts = Vec::new();
    for key in [
        "label",
        "name",
        "aliases",
        "alias",
        "path",
        "document_id",
        "revision_id",
    ] {
        match node.get(key) {
            Some(Value::String(s)) if !s.is_empty() => parts.push(s.clone()),
            Some(Value::Array(values)) => parts.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }
    parts.join(" ")
}

fn node_anchor_text(node: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["label", "name", "aliases", "alias"] {
        match node.get(key) {
            Some(Value::String(s)) if !s.is_empty() => parts.push(s.clone()),
            Some(Value::Array(values)) => parts.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }
    parts.join(" ")
}

fn has_entity_anchor(node: &Value, entity_terms: &[&str]) -> bool {
    if entity_terms.is_empty() {
        return false;
    }
    let anchor = node_anchor_text(node).to_lowercase();
    // A shared workspace/path label is location evidence, not brand/project
    // identity. It must not select the named entity by itself.
    if anchor.contains("workspace:") || anchor.contains('/') {
        return false;
    }
    entity_terms.iter().any(|term| anchor.contains(term))
}

/// Check if query expresses asset-design or asset-edit intent.
pub fn has_asset_edit_intent(q: &str) -> bool {
    let lower = q.to_lowercase();
    lower.contains("重新造字")
        || lower.contains("造字")
        || lower.contains("字體")
        || lower.contains("標準字")
        || lower.contains("logo")
        || lower.contains("wordmark")
        || lower.contains("brand asset")
        || lower.contains("redraw")
        || lower.contains("vector")
        || lower.contains("向量")
        || lower.contains("標誌")
}

/// Tokenize natural language and hyphenated strings into meaningful search terms.
/// Excludes generic single CJK characters and common stop particles to prevent pollution.
pub fn extract_query_terms(q: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let jieba = jieba_rs::Jieba::new();
    for token in jieba.tokenize(q, jieba_rs::TokenizeMode::Search, true) {
        let trimmed = token.word.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Exclude generic particles and single CJK characters
        if trimmed.chars().count() < 2 && !trimmed.is_ascii() {
            continue;
        }
        if matches!(
            trimmed,
            "的" | "了"
                | "在"
                | "是"
                | "我"
                | "有"
                | "和"
                | "就"
                | "不"
                | "人"
                | "都"
                | "一"
                | "個"
                | "上"
                | "也"
                | "很"
                | "到"
                | "說"
                | "要"
                | "去"
                | "妳"
                | "會"
                | "著"
                | "沒有"
                | "看好"
                | "自己"
                | "這"
                | "請將"
                | "將"
                | "粗細"
                | "the"
                | "a"
                | "an"
                | "and"
                | "or"
                | "to"
                | "of"
                | "in"
        ) {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if !terms.contains(&lower) {
            terms.push(lower);
        }
    }

    // Extract raw tokens splitting on whitespace and punctuation
    let raw_tokens: Vec<&str> = q
        .split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ',' | '，'
                        | '.'
                        | '。'
                        | '!'
                        | '！'
                        | '?'
                        | '？'
                        | ';'
                        | '；'
                        | ':'
                        | '：'
                        | '"'
                        | '\''
                        | '、'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                )
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    for t in raw_tokens {
        let lower = t.to_lowercase();
        // Avoid single CJK character terms
        if lower.chars().count() < 2 && !lower.is_ascii() {
            continue;
        }
        if lower.len() >= 2 && !terms.contains(&lower) {
            terms.push(lower.clone());
        }
        // For hyphenated or underscore-joined tokens like `weemed-yi-jian`, also extract parts
        if t.contains('-') || t.contains('_') {
            for sub in t.split(['-', '_']).filter(|s| !s.is_empty()) {
                let sub_lower = sub.to_lowercase();
                if sub_lower.len() >= 2 && !terms.contains(&sub_lower) {
                    terms.push(sub_lower);
                }
            }
        }
    }
    terms
}

/// Sanitize query for Tantivy QueryParser so hyphens and syntax characters
/// do not trigger boolean NOT or syntax errors.
fn sanitize_for_tantivy(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    for c in q.chars() {
        if matches!(
            c,
            '+' | '-'
                | '!'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '^'
                | '"'
                | '~'
                | '*'
                | '?'
                | ':'
                | '\\'
                | '/'
                | '&'
                | '|'
        ) {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Extract heuristic operation cues from the user's natural query.
fn extract_operation_cue(q: &str) -> Option<&'static str> {
    let lower = q.to_lowercase();
    if lower.contains("重新造字") || lower.contains("造字") || lower.contains("redraw") {
        Some("redraw_glyphs")
    } else if lower.contains("修改") || lower.contains("調整") || lower.contains("modify") {
        Some("modify_asset")
    } else if lower.contains("建立") || lower.contains("create") {
        Some("create_asset")
    } else {
        None
    }
}

/// Extract heuristic constraint cues from the user's natural query.
fn extract_constraint_cues(q: &str) -> Vec<String> {
    let mut constraints = Vec::new();
    if q.contains("粗細要一致") || q.contains("粗細一致") {
        constraints.push("粗細要一致".to_string());
    }
    constraints
}

/// Check if a filename is an editable vector/code baseline.
fn is_editable_format(label: &str, path: &str) -> bool {
    let target = if !label.is_empty() { label } else { path };
    let lower = target.to_lowercase();
    lower.ends_with(".svg")
        || lower.ends_with(".ai")
        || lower.ends_with(".eps")
        || lower.ends_with(".typ")
        || lower.ends_with(".fig")
        || lower.ends_with(".sketch")
}

/// Candidate asset item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateItem {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub score: f64,
    pub status: String,
    pub why: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<Value>>,
}

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
    // design, so only retain its rows when an identity-bearing field matches
    // a meaningful query term; body-only generic matches are context noise.
    let q_lower = options.q.trim().to_lowercase();
    let mut candidate_scores: BTreeMap<String, (f64, String)> = BTreeMap::new(); // id -> (score, why)

    for (id, fts_score) in &fts_scores {
        let Some(node) = materialized.nodes.get(id) else {
            continue;
        };
        let searchable = node_searchable_text(node).to_lowercase();
        let admitted = if entity_terms.is_empty() {
            identity_terms.iter().any(|term| searchable.contains(term))
        } else {
            has_entity_anchor(node, &entity_terms)
        };
        if admitted {
            candidate_scores.insert(id.clone(), (*fts_score, "fts_identity_match".to_string()));
        }
    }

    if !q_lower.is_empty() {
        for (id, node) in &materialized.nodes {
            let label = node_text(node, "label").to_lowercase();
            let id_lower = id.to_lowercase();
            let identity = node_identity_text(node).to_lowercase();
            let path_like_label = label.contains("workspace:") || label.contains('/');

            let mut literal_matched = false;
            let mut exact_match = false;

            // Check exact query match (min 2 chars)
            if q_lower.chars().count() >= 2
                && (label.contains(&q_lower) || (!path_like_label && id_lower.contains(&q_lower)))
            {
                literal_matched = true;
                exact_match = true;
            }

            // Check hyphenated tokens or extracted significant terms
            for term in &terms {
                if !entity_terms.is_empty() && !entity_terms.contains(&term.as_str()) {
                    continue;
                }
                if term.len() >= 2
                    && !path_like_label
                    && (label.contains(term) || id_lower.contains(term))
                {
                    literal_matched = true;
                    if label == *term || id_lower == *term {
                        exact_match = true;
                    }
                }
            }

            let identity_match = if entity_terms.is_empty() {
                identity_terms.iter().any(|term| identity.contains(term))
            } else {
                has_entity_anchor(node, &entity_terms)
            };
            if identity_match {
                literal_matched = true;
            }

            if literal_matched {
                let score = if exact_match { 1.0 } else { 0.85 };
                let current = candidate_scores
                    .entry(id.clone())
                    .or_insert((0.0, String::new()));
                if score > current.0 {
                    current.0 = score;
                    current.1 = "literal_match".to_string();
                }
            }
        }
    }

    // 5. Rank candidate seeds (preserving BM25 / literal relevance)
    let mut ranked_candidates: Vec<(String, f64, String)> = candidate_scores
        .into_iter()
        .map(|(id, (score, why))| (id, score, why))
        .collect();
    ranked_candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // 6. Bounded, typed relationship traversal from candidates
    let mut linked_projects: BTreeSet<String> = BTreeSet::new();
    let mut connected_decisions: Vec<Value> = Vec::new();
    let mut candidate_status_map: BTreeMap<String, String> = BTreeMap::new();
    let mut approved_candidates: Vec<(String, String, String)> = Vec::new(); // (id, label, status)

    for (id, _, _) in &ranked_candidates {
        let node = &materialized.nodes[id];
        let raw_status = node_text(node, "status");
        let label = node_text(node, "label");
        let path = node_text(node, "path");

        // Missing status must be unknown, never fabricated draft
        let display_status = if raw_status.trim().is_empty() {
            "unknown".to_string()
        } else {
            raw_status.to_string()
        };
        candidate_status_map.insert(id.clone(), display_status.clone());

        // Check explicit approval (status=approved; not just final, and has verified location)
        let has_explicit_approval = display_status.eq_ignore_ascii_case("approved");
        let has_source_location = node
            .get("location_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || node
                .get("location_verified")
                .and_then(Value::as_str)
                .is_some_and(|v| v.eq_ignore_ascii_case("verified"));

        if has_explicit_approval && is_editable_format(label, path) && has_source_location {
            approved_candidates.push((id.clone(), label.to_string(), display_status));
        }

        // Inspect edges from candidate: ONLY explicit BELONGS_TO edges to a Project node
        for edge in &materialized.edges {
            if edge.retracted {
                continue;
            }
            if edge.from == *id && edge.edge == EdgeType::BelongsTo {
                if let Some(to_node) = materialized.nodes.get(&edge.to) {
                    if node_text(to_node, "type").eq_ignore_ascii_case("project") {
                        linked_projects.insert(edge.to.clone());
                    }
                }
            }
            if (edge.from == *id || edge.to == *id)
                && matches!(edge.edge, EdgeType::Supports | EdgeType::Informs)
            {
                let other_id = if edge.from == *id {
                    &edge.to
                } else {
                    &edge.from
                };
                if let Some(dec_node) = materialized.nodes.get(other_id) {
                    if node_text(dec_node, "type").eq_ignore_ascii_case("decision") {
                        connected_decisions.push(json!({
                            "id": other_id,
                            "label": node_text(dec_node, "label"),
                            "status": node_text(dec_node, "status"),
                        }));
                    }
                }
            }
        }
    }

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

/// Render human-readable output for the task-context entry.
pub fn render(value: &Value) -> String {
    let mut out = String::new();
    let resolution = value["resolution"].as_str().unwrap_or("unknown");
    out.push_str(&format!(
        "Task Context Resolution: [{}]\n",
        resolution.to_uppercase()
    ));
    out.push_str(&format!(
        "User Request: {}\n",
        value["user_request"].as_str().unwrap_or("")
    ));

    if let Some(op) = value["operation_cue"].as_str() {
        out.push_str(&format!("Operation Cue: {op} (heuristic hypothesis)\n"));
    }

    if value["baseline"].is_null() {
        out.push_str("Baseline: none (no approved editable baseline recorded)\n");
    } else {
        out.push_str(&format!(
            "Baseline: {} ({})\n",
            value["baseline"]["label"].as_str().unwrap_or(""),
            value["baseline"]["status"].as_str().unwrap_or("")
        ));
    }

    if let Some(projects) = value["linked_projects"].as_array() {
        if !projects.is_empty() {
            let p_str: Vec<&str> = projects.iter().filter_map(|v| v.as_str()).collect();
            out.push_str(&format!("Linked Projects: {}\n", p_str.join(", ")));
        } else {
            out.push_str("Linked Projects: none\n");
        }
    }

    out.push('\n');
    let total = value["total"].as_u64().unwrap_or(0);
    let next_offset = &value["next_offset"];
    out.push_str(&format!(
        "Candidate Assets: {total} (next_offset={next_offset})\n"
    ));

    if let Some(rows) = value["rows"].as_array() {
        for row in rows {
            out.push_str(&format!(
                "  {} | {} | {} | score: {:.2} | status: {}\n",
                row[0].as_str().unwrap_or(""),
                row[1].as_str().unwrap_or(""),
                row[2].as_str().unwrap_or(""),
                row[3].as_f64().unwrap_or(0.0),
                row[4].as_str().unwrap_or("")
            ));
        }
    } else if let Some(candidates) = value["candidates"].as_array() {
        for c in candidates {
            out.push_str(&format!(
                "  {} | {} | {} | score: {:.2} | status: {}\n",
                c["id"].as_str().unwrap_or(""),
                c["kind"].as_str().unwrap_or(""),
                c["label"].as_str().unwrap_or(""),
                c["score"].as_f64().unwrap_or(0.0),
                c["status"].as_str().unwrap_or("")
            ));
            if let Some(history) = c["history"].as_array() {
                out.push_str(&format!("    History ({} events):\n", history.len()));
                for ev in history {
                    out.push_str(&format!(
                        "      line {:>3} [{}] {} {}\n",
                        ev["line"].as_u64().unwrap_or(0),
                        ev["at"].as_str().unwrap_or(""),
                        ev["op"].as_str().unwrap_or(""),
                        ev["event"].as_str().unwrap_or("")
                    ));
                }
            }
        }
    }

    if let Some(missing) = value["missing_evidence"].as_array() {
        if !missing.is_empty() {
            out.push_str("\nMissing Evidence:\n");
            for m in missing {
                out.push_str(&format!("  - {}\n", m.as_str().unwrap_or("")));
            }
        }
    }

    if let Some(exp) = value["evidence_expansion"]["argv"].as_array() {
        let argv_str: Vec<&str> = exp.iter().filter_map(|v| v.as_str()).collect();
        out.push_str(&format!("\nEvidence Expansion: {}\n", argv_str.join(" ")));
    }

    out
}
