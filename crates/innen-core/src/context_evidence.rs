//! Evidence descriptors, not inferred world facts. Source roles and reported
//! process status must never be promoted to successful task/validation outcomes.
use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct EvidenceInput {
    pub id: u64,
    pub role: String,
    pub text: String,
    pub timestamp: Option<String>,
    pub record_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Serialize)]
pub struct ProcessReport {
    /// The exporter reports this code; it is not an independent rerun result.
    pub exit_code: i32,
    pub source: Span,
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub event: u64,
    pub origin: String,
    pub source_timestamp: Option<String>,
    pub source_record_status: Option<String>,
    pub text_sha256: String,
    pub text: Span,
    pub process_report: Option<ProcessReport>,
    pub operation_candidate: Option<OperationCandidate>,
    /// Literal negative-marker lines, including possible quotes or source code.
    /// These are NOT validated failure observations.
    pub diagnostic_mentions: Vec<Span>,
    pub task_outcome: &'static str,
    pub subject: Option<String>,
    pub scope: Option<String>,
    pub negation_scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OperationCandidate {
    pub event: u64,
    pub linkage: &'static str,
}

/// Export formats without call IDs only support a temporal candidate. Keep the
/// invocation available, but never imply that adjacency proves correlation.
pub fn describe_sequence(inputs: Vec<EvidenceInput>) -> Vec<Evidence> {
    let mut prior_dispatch = None;
    inputs
        .into_iter()
        .map(|input| {
            let next_dispatch = (input.role == "tool_dispatch").then_some(input.id);
            let candidate = (input.role == "tool").then_some(prior_dispatch).flatten();
            let mut result = describe(input);
            result.operation_candidate = candidate.map(|event| OperationCandidate {
                event,
                linkage: "adjacent_dispatch_only_not_verified_correlation",
            });
            prior_dispatch = next_dispatch;
            result
        })
        .collect()
}

fn report(text: &str) -> Option<ProcessReport> {
    let mut offset = 0;
    let mut rows = text.split_inclusive('\n').peekable();
    // Recognize the known export envelope only at the beginning. Never scan
    // arbitrary body text for a status that may have been quoted there.
    if rows
        .peek()
        .is_some_and(|line| line.starts_with("Created At: "))
    {
        offset += rows.next()?.len();
        if !rows.peek()?.starts_with("Completed At: ") {
            return None;
        }
        offset += rows.next()?.len();
    }
    while rows.peek().is_some_and(|line| line.trim().is_empty()) {
        offset += rows.next()?.len();
    }
    let line = rows.next()?;
    let digits = line
        .trim_end()
        .strip_prefix("The command exited with code ")?
        .strip_suffix('.')?;
    let code = digits.parse::<i32>().ok()?;
    Some(ProcessReport {
        exit_code: code,
        source: Span {
            start_byte: offset,
            end_byte: offset + line.trim_end().len(),
        },
    })
}

fn negative_marker(line: &str) -> bool {
    line.split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| matches!(word, "FAIL" | "FAILED" | "ERROR"))
}

pub fn describe(input: EvidenceInput) -> Evidence {
    let origin = match input.role.as_str() {
        "user" => "user_text_not_necessarily_a_request",
        "assistant" => "assistant_statement_not_verified_fact",
        "tool" => "tool_record_not_task_outcome",
        "system" => "historical_system_text",
        "tool_dispatch" => "attempted_operation_not_result",
        _ => "unresolved_source_role",
    }
    .to_string();
    let process_report = (input.role == "tool")
        .then(|| report(&input.text))
        .flatten();
    let mut diagnostic_mentions = Vec::new();
    let mut offset = 0;
    if input.role == "tool" {
        for line in input.text.split_inclusive('\n') {
            if negative_marker(line) {
                diagnostic_mentions.push(Span {
                    start_byte: offset,
                    end_byte: offset + line.trim_end().len(),
                });
            }
            offset += line.len();
        }
    }
    Evidence {
        event: input.id,
        origin,
        source_timestamp: input.timestamp,
        source_record_status: input.record_status,
        text_sha256: sha256_hex(input.text.as_bytes()),
        text: Span {
            start_byte: 0,
            end_byte: input.text.len(),
        },
        process_report,
        operation_candidate: None,
        diagnostic_mentions,
        task_outcome: "not_inferred",
        subject: None,
        scope: None,
        negation_scope: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(role: &str, text: &str) -> EvidenceInput {
        EvidenceInput {
            id: 1,
            role: role.into(),
            text: text.into(),
            timestamp: None,
            record_status: Some("DONE".into()),
        }
    }

    #[test]
    fn outer_success_and_negative_body_do_not_become_task_success() {
        let text = "Created At: now\nCompleted At: later\n\nThe command exited with code 0.\nOutput:\nUI EVIDENCE FAIL: mismatch\n";
        let e = describe(input("tool", text));
        assert_eq!(e.process_report.as_ref().unwrap().exit_code, 0);
        assert_eq!(e.diagnostic_mentions.len(), 1);
        assert_eq!(e.task_outcome, "not_inferred");
        assert_eq!(
            &text[e.diagnostic_mentions[0].start_byte..e.diagnostic_mentions[0].end_byte],
            "UI EVIDENCE FAIL: mismatch"
        );
    }

    #[test]
    fn copied_status_in_assistant_or_body_is_not_exporter_status() {
        assert!(
            describe(input("assistant", "The command exited with code 0."))
                .process_report
                .is_none()
        );
        assert!(describe(input(
            "tool",
            "File Path: example\nThe command exited with code 0."
        ))
        .process_report
        .is_none());
        let e = describe(input(
            "tool",
            "The command exited with code 0.\nOutput:\nprint(\"FAIL\")\n",
        ));
        assert_eq!(e.diagnostic_mentions.len(), 1);
        assert_eq!(e.task_outcome, "not_inferred");
    }

    #[test]
    fn negative_process_report_is_preserved_and_unknowns_stay_unknown() {
        let e = describe(input("tool", "The command exited with code 1.\n"));
        assert_eq!(e.process_report.unwrap().exit_code, 1);
        assert!(e.subject.is_none() && e.scope.is_none() && e.negation_scope.is_none());
        assert_eq!(e.task_outcome, "not_inferred");
    }

    #[test]
    fn invocation_link_is_only_adjacent_and_never_claims_correlation() {
        let mut call = input("tool_dispatch", "git grep FAIL");
        call.id = 10;
        let output = describe_sequence(vec![
            call,
            input("tool", "The command exited with code 0.\nFAIL\n"),
            input("tool", "later async result"),
        ]);
        assert_eq!(output[1].operation_candidate.as_ref().unwrap().event, 10);
        assert_eq!(
            output[1].operation_candidate.as_ref().unwrap().linkage,
            "adjacent_dispatch_only_not_verified_correlation"
        );
        assert!(output[2].operation_candidate.is_none());
    }
}
