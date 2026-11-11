//! Narrative arc extraction for presentations/docs.

use super::discourse::{DiscourseUnit, Role};

/// Extract narrative arc from conversation flow: Why → Scenario → Vehicle → Architecture.
pub fn extract_narrative_arc_from_flow(units: &[DiscourseUnit]) -> Option<super::NarrativeArc> {
    let mut arc = super::NarrativeArc {
        why: None,
        scenario: None,
        vehicle: None,
        architecture: None,
        source_ranges: super::NarrativeRanges::default(),
    };
    let mut found = false;

    for (idx, unit) in units.iter().enumerate() {
        let content_lower = unit.content.to_lowercase();

        if arc.why.is_none() && unit.role == Role::User {
            if content_lower.contains("why")
                || content_lower.contains("motivation")
                || content_lower.contains("problem")
                || content_lower.contains("context")
                || content_lower.contains("background")
                || content_lower.contains("reason")
                || content_lower.contains("在社區到底要做什麼")
                || content_lower.contains("上次提報沒過主因")
                || content_lower.contains("個資")
                || content_lower.contains("合法")
                || content_lower.contains("合規")
            {
                arc.why = Some(extract_section(units, idx, 30));
                arc.source_ranges.why = Some((unit.line, unit.line + 100));
                found = true;
            }
        }

        if arc.scenario.is_none() {
            if content_lower.contains("scenario")
                || content_lower.contains("use case")
                || content_lower.contains("情境")
                || content_lower.contains("情況")
                || content_lower.contains("situation")
                || content_lower.contains("example")
                || content_lower.contains("例如")
                || content_lower.contains("比如")
            {
                arc.scenario = Some(extract_section(units, idx, 20));
                arc.source_ranges.scenario = Some((unit.line, unit.line + 80));
                found = true;
            }
        }

        if arc.vehicle.is_none() {
            if content_lower.contains("format")
                || content_lower.contains("vehicle")
                || content_lower.contains("delivery")
                || content_lower.contains("interface")
                || content_lower.contains("載具")
                || content_lower.contains("簡報")
                || content_lower.contains("投影片")
                || content_lower.contains("slide")
                || content_lower.contains("pptx")
                || content_lower.contains("pdf")
                || content_lower.contains("google slides")
                || content_lower.contains("openslide")
            {
                arc.vehicle = Some(extract_section(units, idx, 20));
                arc.source_ranges.vehicle = Some((unit.line, unit.line + 80));
                found = true;
            }
        }

        if arc.architecture.is_none() && unit.role == Role::Assistant {
            if content_lower.contains("architecture")
                || content_lower.contains("architect")
                || content_lower.contains("架構")
                || content_lower.contains("設計")
                || content_lower.contains("design")
                || content_lower.contains("system")
                || content_lower.contains("中繼")
                || content_lower.contains("vpn")
                || content_lower.contains("ssot")
                || content_lower.contains("資安")
                || content_lower.contains("公鑰")
                || content_lower.contains("數位信封")
                || content_lower.contains("hash")
                || content_lower.contains("sha")
            {
                arc.architecture = Some(extract_section(units, idx, 40));
                arc.source_ranges.architecture = Some((unit.line, unit.line + 120));
                found = true;
            }
        }
    }

    if found {
        Some(arc)
    } else {
        None
    }
}

fn extract_section(units: &[DiscourseUnit], start_idx: usize, window: usize) -> String {
    let mut content = String::new();
    let end = (start_idx + window).min(units.len());
    for unit in &units[start_idx..end] {
        if !unit.content.is_empty() && unit.role != Role::System {
            content.push_str(&unit.content);
            content.push('\n');
        }
    }
    truncate(&content, 1000)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
