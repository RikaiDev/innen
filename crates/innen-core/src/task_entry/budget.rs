//! A size ceiling for the context brief. Past it, the brief gives up detail in
//! a fixed order and says what it gave up, so the agent expands only what it
//! needs (`--view evidence`, `--offset`) instead of paying for all of it.

use serde_json::{json, Value};

/// Detail given up in order until the brief fits: `text` shortens constraint
/// wording to that many characters, a list key keeps that many entries.
const LADDER: [(&str, usize); 8] = [
    ("text", 120),
    ("neighborhood", 3),
    ("constraints", 4),
    ("rows", 5),
    ("text", 60),
    ("neighborhood", 0),
    ("constraints", 2),
    ("rows", 3),
];

fn size(v: &Value) -> usize {
    v.to_string().chars().count()
}

fn keep(out: &mut Value, key: &str, n: usize) -> Option<(usize, usize)> {
    let items = out[key].as_array_mut().filter(|a| a.len() > n)?;
    let before = items.len();
    items.truncate(n);
    Some((before, n))
}

fn shorten(out: &mut Value, chars: usize) -> bool {
    let mut changed = false;
    for rule in out["constraints"].as_array_mut().into_iter().flatten() {
        let text = rule["text"].as_str().unwrap_or("");
        if text.chars().count() > chars {
            let short: String = text.chars().take(chars).collect();
            rule["text"] = json!(format!("{short}…"));
            changed = true;
        }
    }
    changed
}

/// Give up detail in `LADDER` order and report what was given up. The request,
/// resolution, baseline, projects, missing evidence and the expansion command
/// are never cut; `fits` says whether the ceiling was reached.
pub(super) fn fit_to_budget(out: &mut Value, budget: usize, offset: usize) {
    let mut cut: Vec<String> = Vec::new();
    for (key, n) in LADDER {
        if size(out) <= budget {
            break;
        }
        if key == "text" {
            if shorten(out, n) {
                cut.push(format!("constraint text shortened to {n} characters"));
            }
        } else if let Some((before, after)) = keep(out, key, n) {
            cut.push(format!("{key} {before}->{after}"));
            if key == "rows" {
                out["next_offset"] = json!(offset + after);
            }
        }
    }
    if !cut.is_empty() {
        out["budget"] = json!({"chars": budget, "fits": size(out) <= budget, "cut": cut});
    }
}
