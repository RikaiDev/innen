use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct SweepItem {
    pub session_id: String,
    pub source: String,
    pub status: String,
    pub source_bytes: u64,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepReceipt {
    pub schema: &'static str,
    pub total: usize,
    pub eligible: usize,
    pub blocked: usize,
    pub removed: usize,
    pub failed: usize,
    pub native_bytes_removed: u64,
    pub executed: bool,
    pub items: Vec<SweepItem>,
}

pub fn sweep(
    root: &Path,
    source: Option<Source>,
    source_root: Option<&Path>,
    retention_days: u64,
    execute: bool,
) -> Result<SweepReceipt, Error> {
    let assessments = inventory(root, source, source_root, retention_days)?;
    let mut receipt = SweepReceipt {
        schema: "innen.session-retention-sweep.v1",
        total: assessments.len(),
        eligible: assessments.iter().filter(|item| item.eligible).count(),
        blocked: assessments.iter().filter(|item| !item.eligible).count(),
        removed: 0,
        failed: 0,
        native_bytes_removed: 0,
        executed: execute,
        items: Vec::new(),
    };
    for assessment in assessments.into_iter().filter(|item| item.eligible) {
        if !execute {
            receipt.items.push(SweepItem {
                session_id: assessment.session_id,
                source: assessment.source,
                status: "eligible".into(),
                source_bytes: assessment.source_bytes,
                detail: None,
            });
            continue;
        }
        match purge(
            root,
            &assessment.source,
            &assessment.session_id,
            source_root,
            retention_days,
            true,
        ) {
            Ok(_) => {
                receipt.removed += 1;
                receipt.native_bytes_removed = receipt
                    .native_bytes_removed
                    .saturating_add(assessment.source_bytes);
                receipt.items.push(SweepItem {
                    session_id: assessment.session_id,
                    source: assessment.source,
                    status: "removed".into(),
                    source_bytes: assessment.source_bytes,
                    detail: None,
                });
            }
            Err(error) => {
                receipt.failed += 1;
                receipt.items.push(SweepItem {
                    session_id: assessment.session_id,
                    source: assessment.source,
                    status: "failed".into(),
                    source_bytes: assessment.source_bytes,
                    detail: Some(error.to_string()),
                });
            }
        }
    }
    Ok(receipt)
}
