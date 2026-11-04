//! Information-theoretic helpers for the brief. A term few nodes contain says
//! more than a common one, and a document kept in two formats is one fact.

use std::collections::HashMap;

use serde_json::Value;

use super::node_identity::{node_identity_text, node_kind, node_text};

/// Self-information of each term over the identity text of every node:
/// `ln((N+1)/(df+1)) + 1`. A rare term outweighs a common one, and a term no
/// node holds still counts.
pub(super) fn term_weights(nodes: &HashMap<String, Value>, terms: &[String]) -> Vec<f64> {
    let lowered: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let mut df = vec![0usize; lowered.len()];
    for node in nodes.values() {
        let hay = node_identity_text(node).to_lowercase();
        for (i, t) in lowered.iter().enumerate() {
            if hay.contains(t.as_str()) {
                df[i] += 1;
            }
        }
    }
    let n = nodes.len() as f64;
    df.iter()
        .map(|d| ((n + 1.0) / (*d as f64 + 1.0)).ln() + 1.0)
        .collect()
}

/// `name.pdf` -> (`name`, Some(`pdf`)); a label without a short alphanumeric
/// extension is its own stem.
fn split_ext(label: &str) -> (&str, Option<&str>) {
    match label.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (stem, Some(ext))
        }
        _ => (label, None),
    }
}

/// One shown entry standing for every node that carries the same document.
pub(super) struct Folded {
    pub id: String,
    pub label: String,
    /// The other node ids folded into this entry, kept so they can be expanded.
    pub also: Vec<String>,
}

/// Fold nodes of one type that share a stem into one entry, in first-seen
/// order: `x.pptx` + `x.pdf` become `x.{pptx,pdf}`, and one label repeated by
/// several revisions becomes `x.pdf ×3`. Nodes without an extension never fold.
pub(super) fn fold_documents(items: &[(String, String, String)]) -> Vec<Folded> {
    let mut out: Vec<Folded> = Vec::new();
    let mut groups: HashMap<(String, String), (usize, Vec<String>, usize)> = HashMap::new();
    for (id, kind, label) in items {
        let (stem, ext) = split_ext(label);
        let Some(ext) = ext else {
            out.push(Folded {
                id: id.clone(),
                label: label.clone(),
                also: Vec::new(),
            });
            continue;
        };
        match groups.get_mut(&(kind.clone(), stem.to_string())) {
            Some((at, exts, count)) => {
                if !exts.iter().any(|e| e == ext) {
                    exts.push(ext.to_string());
                }
                *count += 1;
                out[*at].also.push(id.clone());
                out[*at].label = folded_label(stem, exts, *count);
            }
            None => {
                groups.insert(
                    (kind.clone(), stem.to_string()),
                    (out.len(), vec![ext.to_string()], 1),
                );
                out.push(Folded {
                    id: id.clone(),
                    label: label.clone(),
                    also: Vec::new(),
                });
            }
        }
    }
    out
}

fn folded_label(stem: &str, exts: &[String], count: usize) -> String {
    match (exts.len(), count) {
        (1, n) => format!("{stem}.{} ×{n}", exts[0]),
        _ => format!("{stem}.{{{}}}", exts.join(",")),
    }
}

/// Two constraint texts that say the same thing: their character bigrams
/// overlap almost entirely, so the second adds no information.
pub(super) fn near_duplicate(a: &str, b: &str) -> bool {
    fn grams(s: &str) -> std::collections::HashSet<(char, char)> {
        let c: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    }
    let (ga, gb) = (grams(a), grams(b));
    if ga.is_empty() || gb.is_empty() {
        return a.trim() == b.trim();
    }
    let both = ga.intersection(&gb).count() as f64;
    both / (ga.len() + gb.len()) as f64 * 2.0 >= 0.85
}

/// Ranked candidates with same-document nodes folded into the best-ranked one:
/// the shortened list, the label to show per kept id, and the folded ids that
/// were merged into it.
pub(super) type FoldedRanking = (
    Vec<(String, f64, String)>,
    HashMap<String, String>,
    std::collections::BTreeMap<String, Vec<String>>,
);

pub(super) fn fold_ranked(
    ranked: Vec<(String, f64, String)>,
    nodes: &HashMap<String, Value>,
) -> FoldedRanking {
    let items: Vec<(String, String, String)> = ranked
        .iter()
        .map(|(id, _, _)| {
            let node = &nodes[id];
            (
                id.clone(),
                node_kind(node),
                node_text(node, "label").to_string(),
            )
        })
        .collect();
    let by_id: HashMap<&str, &(String, f64, String)> =
        ranked.iter().map(|r| (r.0.as_str(), r)).collect();
    let mut kept = Vec::new();
    let mut shown = HashMap::new();
    let mut folded = std::collections::BTreeMap::new();
    for f in fold_documents(&items) {
        kept.push(by_id[f.id.as_str()].clone());
        shown.insert(f.id.clone(), f.label);
        if !f.also.is_empty() {
            folded.insert(f.id, f.also);
        }
    }
    (kept, shown, folded)
}

const LABEL_CLIP: usize = 80;
const WEAK_SCORE_RATIO: f64 = 0.5;
const MIN_KEPT: usize = 3;

/// A label longer than `LABEL_CLIP` characters is cut; the full text stays in
/// the evidence view.
pub(super) fn clip_label(label: &str) -> String {
    if label.chars().count() <= LABEL_CLIP {
        return label.to_string();
    }
    let cut: String = label.chars().take(LABEL_CLIP).collect();
    format!("{cut}…")
}

/// Drop candidates that add nothing: ids already listed elsewhere in the brief,
/// and, for a per-term fallback, matches scoring under half the best score
/// (keeping at least `MIN_KEPT`). Input is best-first.
pub(super) fn prune_candidates(
    ranked: Vec<(String, f64, String)>,
    already_shown: &std::collections::HashSet<String>,
    per_term: bool,
) -> Vec<(String, f64, String)> {
    let fresh: Vec<(String, f64, String)> = ranked
        .iter()
        .filter(|r| !already_shown.contains(&r.0))
        .cloned()
        .collect();
    let base = if fresh.is_empty() { ranked } else { fresh };
    if !per_term {
        return base;
    }
    let top = base.first().map_or(0.0, |r| r.1);
    base.iter()
        .enumerate()
        .filter(|(i, r)| *i < MIN_KEPT || r.1 >= top * WEAK_SCORE_RATIO)
        .map(|(_, r)| r.clone())
        .collect()
}

pub(super) fn constraint_ids(constraints: &[Value]) -> std::collections::HashSet<String> {
    constraints
        .iter()
        .filter_map(|c| c["id"].as_str().map(str::to_string))
        .collect()
}

/// A term the user's recorded words contain is not missing evidence.
pub(super) fn drop_accounted_terms(unmatched: &mut Vec<String>, constraints: &[Value]) {
    unmatched.retain(|t| {
        let t = t.to_lowercase();
        !constraints.iter().any(|c| {
            c["text"]
                .as_str()
                .is_some_and(|x| x.to_lowercase().contains(&t))
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, label: &str) -> (String, String, String) {
        (id.into(), "Artifact".into(), label.into())
    }

    #[test]
    fn same_stem_documents_fold_into_one_entry() {
        let out = fold_documents(&[item("a", "x.pptx"), item("b", "y.md"), item("c", "x.pdf")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].label, "x.{pptx,pdf}");
        assert_eq!(out[0].also, vec!["c".to_string()]);
        assert_eq!(out[1].label, "y.md");
    }

    #[test]
    fn repeated_label_shows_a_count_and_labels_without_extension_never_fold() {
        let out = fold_documents(&[
            item("a", "x.pdf"),
            item("b", "x.pdf"),
            item("c", "plain"),
            item("d", "plain"),
        ]);
        assert_eq!(out[0].label, "x.pdf ×2");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn rare_terms_weigh_more_than_common_ones() {
        let mut nodes = HashMap::new();
        for i in 0..9 {
            nodes.insert(format!("n{i}"), json!({"label": "共通 詞"}));
        }
        nodes.insert("r".into(), json!({"label": "共通 稀有"}));
        let w = term_weights(&nodes, &["共通".to_string(), "稀有".to_string()]);
        assert!(w[1] > w[0], "{w:?}");
    }

    #[test]
    fn near_duplicates_are_detected_but_distinct_texts_are_not() {
        assert!(near_duplicate(
            "上次資委會沒過，原因是個資",
            "上次資委會沒過，原因是個資！"
        ));
        assert!(!near_duplicate(
            "上次資委會沒過，原因是個資",
            "簡報請用 open-slide 製作"
        ));
    }

    #[test]
    fn pruning_drops_shown_ids_and_weak_per_term_matches() {
        let r = |id: &str, s: f64| (id.to_string(), s, String::new());
        let ranked = vec![
            r("a", 1.0),
            r("b", 0.9),
            r("c", 0.8),
            r("d", 0.7),
            r("e", 0.1),
        ];
        let shown = std::collections::HashSet::from(["b".to_string()]);
        let kept: Vec<String> = prune_candidates(ranked.clone(), &shown, true)
            .into_iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(kept, ["a", "c", "d"]);
        assert_eq!(prune_candidates(ranked, &shown, false).len(), 4);
    }

    #[test]
    fn long_labels_are_clipped() {
        assert_eq!(clip_label("短"), "短");
        assert_eq!(
            clip_label(&"字".repeat(200)).chars().count(),
            LABEL_CLIP + 1
        );
    }
}
