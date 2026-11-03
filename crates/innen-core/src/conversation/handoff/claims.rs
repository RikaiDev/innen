//! Assistant claims a transcript cannot prove: what a third party approved.

use super::{Handoff, Scan, MAX_CLAIMS};

const CLAIM_CONTEXT_CHARS: usize = 30;
const CLAIMS_PER_REPLY: usize = 2;
const MAX_CLAIM_CHARS: usize = 120;

fn find(chars: &[char], needle: &str) -> Option<(usize, usize)> {
    let n: Vec<char> = needle.chars().collect();
    (0..=chars.len().saturating_sub(n.len()))
        .find(|&i| chars[i..].starts_with(&n))
        .map(|i| (i, i + n.len()))
}

/// Earliest hit of any of `needles` in `chars`.
fn first_of(chars: &[char], needles: &[&str]) -> Option<(usize, usize)> {
    needles
        .iter()
        .filter_map(|n| find(chars, n))
        .min_by_key(|span| span.0)
}

/// Earliest `lead` followed, within `window` chars, by any of `then`.
fn lead_then(chars: &[char], lead: &str, then: &[&str], window: usize) -> Option<(usize, usize)> {
    let l: Vec<char> = lead.chars().collect();
    (0..chars.len())
        .filter(|&i| chars[i..].starts_with(&l))
        .find_map(|i| {
            let from = i + l.len();
            let rest = &chars[from..(from + window).min(chars.len())];
            first_of(rest, then).map(|(_, end)| (i, from + end))
        })
}

/// What an agent can honestly say only about the outside world, and which its
/// own tool results cannot confirm: that a third party has approved the work.
/// The sentence must carry both a perfective "has passed / obtained / completed
/// at X" and an approving body, because wording like "審查通過" alone also
/// describes contest rules or product features. Routine "built / fixed / tested"
/// reports are left out: they are common in every transcript, so they carry
/// little information, and the written files and tool results check them.
/// Known gap: claims that work reached somewhere (deployed, submitted) are not
/// detected; those words are ordinary vocabulary in many transcripts.
const APPROVING_BODY: [&str; 7] = ["審查", "查核", "核准", "批准", "認證", "稽核", "驗收"];
const HAS_PASSED: [&str; 6] = ["已通過", "已經通過", "已獲", "已取得", "已核准", "已批准"];
const HAS_PASSED_EN: [&str; 6] = [
    "has passed",
    "have passed",
    "passed review",
    "approved by",
    "has been approved",
    "signed off",
];
const AT_WINDOW: usize = 24;

fn claim_span(sentence: &str) -> Option<(usize, usize)> {
    let lower: Vec<char> = sentence.to_lowercase().chars().collect();
    if let Some(span) = first_of(&lower, &HAS_PASSED_EN) {
        return Some(span);
    }
    let body = first_of(&lower, &APPROVING_BODY)?;
    let done = first_of(&lower, &HAS_PASSED)
        .or_else(|| lead_then(&lower, "已在", &["完成", "通過"], AT_WINDOW))
        .or_else(|| lead_then(&lower, "已於", &["完成", "通過"], AT_WINDOW))?;
    Some((body.0.min(done.0), body.1.max(done.1)))
}

pub(super) fn collect_claims(text: &str, line: usize, scan: &mut Scan, handoff: &mut Handoff) {
    let mut taken = 0;
    for sentence in text.split(['。', '\n']) {
        let clean: String = sentence
            .chars()
            .filter(|c| !matches!(c, '*' | '`' | '#' | '>'))
            .collect();
        let clean = clean.trim();
        let Some((start, end)) = claim_span(clean) else {
            continue;
        };
        let chars: Vec<char> = clean.chars().collect();
        let from = start.saturating_sub(CLAIM_CONTEXT_CHARS);
        let to = (end + CLAIM_CONTEXT_CHARS)
            .min(chars.len())
            .min(from + MAX_CLAIM_CHARS);
        if from >= to {
            continue;
        }
        scan.claims_seen += 1;
        if handoff.assistant_claims.len() < MAX_CLAIMS {
            handoff
                .assistant_claims
                .push((line, chars[from..to].iter().collect()));
        }
        taken += 1;
        if taken == CLAIMS_PER_REPLY {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_third_party_approval_claims_are_kept() {
        assert!(
            claim_span("這套流程已在臺大醫院雲林分院資訊管理委員會完成資安審查與驗證").is_some()
        );
        assert!(claim_span("已通過醫院資安查核的四步落地架構").is_some());
        assert!(claim_span("The design was approved by the board").is_some());
        // Routine self-reports, contest prose and product vocabulary are not.
        assert!(claim_span("已完成編譯，make check 已通過").is_none());
        assert!(claim_span("已完成重構與清理").is_none());
        assert!(claim_span("書面審查通過之入選團隊，核發獎勵金").is_none());
        assert!(claim_span(
            "已於 2026-11-02 全數匯入並歸檔於伺服器，本質上是已完成同步的歷史資料，不需要人工核准"
        )
        .is_none());
        // Known gap, pinned on purpose: "reached somewhere outside" is not detected.
        assert!(claim_span("已正式為您完成投件").is_none());
    }
}
