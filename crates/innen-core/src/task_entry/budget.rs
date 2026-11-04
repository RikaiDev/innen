//! A size ceiling for the context brief, chosen the way a rate-distortion
//! trade-off is: every list entry has a value and a cost in characters, and the
//! entries kept are the ones that carry the most value per character until the
//! ceiling is reached. What is left out is reported, so the agent expands only
//! what it needs (`--view evidence`, `--offset`) instead of paying for all of it.

use serde_json::{json, Value};

const SHORT_TEXT: usize = 60;
const LISTS: [&str; 3] = ["constraints", "neighborhood", "rows"];

fn size(v: &Value) -> usize {
    v.to_string().chars().count()
}

/// Constraints are the user's own words and weigh most; candidate rows weigh by
/// their match score; sibling files weigh least. Each list loses value with
/// rank, since lists arrive best-first.
fn value(list: &str, pos: usize, entry: &Value) -> f64 {
    let decay = 0.85_f64.powi(i32::try_from(pos).unwrap_or(i32::MAX));
    match list {
        "constraints" => decay,
        "rows" => entry[3].as_f64().unwrap_or(0.1).clamp(0.05, 1.0) * 0.8 * decay,
        _ => 0.25 * decay,
    }
}

fn shortened(entry: &Value) -> Value {
    let text = entry["text"].as_str().unwrap_or("");
    let mut out = entry.clone();
    if text.chars().count() > SHORT_TEXT {
        let cut: String = text.chars().take(SHORT_TEXT).collect();
        out["text"] = json!(format!("{cut}…"));
    }
    out
}

/// Keep the highest value-per-character entries that fit. The request,
/// resolution, baseline, projects, missing evidence and expansion are never
/// cut. The best constraint and the best row are always kept, even over the
/// ceiling, and `fits` says whether the ceiling was reached.
pub(super) fn fit_to_budget(out: &mut Value, budget: usize, offset: usize) {
    if size(out) <= budget {
        return;
    }
    let originals: Vec<Vec<Value>> = LISTS
        .iter()
        .map(|k| out[*k].as_array().cloned().unwrap_or_default())
        .collect();
    for k in LISTS {
        if out[k].is_array() {
            out[k] = json!([]);
        }
    }
    let mut used = size(out);
    let mut items: Vec<(usize, usize, f64, usize)> = Vec::new();
    for (li, list) in originals.iter().enumerate() {
        for (pos, entry) in list.iter().enumerate() {
            items.push((li, pos, value(LISTS[li], pos, entry), size(entry) + 1));
        }
    }
    let density = |i: &(usize, usize, f64, usize)| i.2 / i.3 as f64;
    items.sort_by(|a, b| {
        density(b)
            .total_cmp(&density(a))
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });
    let mut chosen: Vec<Vec<Option<Value>>> =
        originals.iter().map(|l| vec![None; l.len()]).collect();
    let mut shortened_count = 0;
    for (li, pos, _, cost) in items {
        let entry = &originals[li][pos];
        if used + cost <= budget {
            chosen[li][pos] = Some(entry.clone());
            used += cost;
        } else if LISTS[li] == "constraints" {
            let short = shortened(entry);
            let short_cost = size(&short) + 1;
            if short_cost < cost && used + short_cost <= budget {
                chosen[li][pos] = Some(short);
                used += short_cost;
                shortened_count += 1;
            }
        }
    }
    for (li, k) in [(0, "constraints"), (2, "rows")] {
        if !originals[li].is_empty() && chosen[li].iter().all(Option::is_none) {
            let first = &originals[li][0];
            chosen[li][0] = Some(if k == "constraints" {
                shortened(first)
            } else {
                first.clone()
            });
        }
    }
    let mut cut: Vec<String> = Vec::new();
    for (li, k) in LISTS.iter().enumerate() {
        let kept: Vec<Value> = if *k == "rows" {
            // rows are paged with --offset, so only a leading run can be kept
            chosen[li].iter().map_while(Clone::clone).collect()
        } else {
            chosen[li].iter().flatten().cloned().collect()
        };
        if kept.len() < originals[li].len() {
            cut.push(format!("{k} {}->{}", originals[li].len(), kept.len()));
            if *k == "rows" {
                out["next_offset"] = json!(offset + kept.len());
            }
        }
        if out[*k].is_array() {
            out[*k] = json!(kept);
        }
    }
    if shortened_count > 0 {
        cut.push(format!(
            "{shortened_count} constraint texts shortened to {SHORT_TEXT} characters"
        ));
    }
    out["budget"] = json!({"chars": budget, "fits": true, "cut": cut});
    let fits = size(out) <= budget;
    out["budget"]["fits"] = json!(fits);
}
