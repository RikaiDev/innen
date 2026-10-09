//! Human-readable rendering of the task-context payload.

use serde_json::Value;

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
