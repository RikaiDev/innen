//! Query-text reading: tokenization into search terms plus the intent,
//! operation, and constraint cues a retrieval is judged against.

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
pub(super) fn sanitize_for_tantivy(q: &str) -> String {
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
pub(super) fn extract_operation_cue(q: &str) -> Option<&'static str> {
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
pub(super) fn extract_constraint_cues(q: &str) -> Vec<String> {
    let mut constraints = Vec::new();
    if q.contains("粗細要一致") || q.contains("粗細一致") {
        constraints.push("粗細要一致".to_string());
    }
    constraints
}
