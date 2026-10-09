//! Internal state: event-log append, state read-back, and projection.

use std::fs::{self, OpenOptions};
use std::io::Write;

use super::fs::{atomic, io};
use super::model::{
    EventRecord, FileEvent, Projection, Receipt, BEGIN, END, EVENTS, MANIFEST, TIMELINE,
};
use super::{Ledger, LedgerError};

impl Ledger {
    pub(super) fn append_locked(&self, x: FileEvent) -> Result<(), LedgerError> {
        let mut f = OpenOptions::new()
            .append(true)
            .open(self.root.join(EVENTS))
            .map_err(io)?;
        let line = serde_json::to_string(&x).map_err(|e| LedgerError::Invalid(e.to_string()))?;
        f.write_all(format!("{line}\n").as_bytes()).map_err(io)?;
        f.sync_all().map_err(io)?;
        self.project()
            .map_err(|e| LedgerError::ProjectionAfterAppend(e.to_string()))
    }
    pub(super) fn read_state(
        &self,
    ) -> Result<(Projection, Vec<Receipt>, Vec<EventRecord>), LedgerError> {
        let p: Projection =
            serde_json::from_slice(&fs::read(self.root.join(MANIFEST)).map_err(io)?)
                .map_err(|e| LedgerError::Invalid(e.to_string()))?;
        let mut rs = vec![];
        let mut es = vec![];
        for line in fs::read_to_string(self.root.join(EVENTS))
            .map_err(io)?
            .lines()
            .filter(|x| !x.trim().is_empty())
        {
            let x: FileEvent =
                serde_json::from_str(line).map_err(|e| LedgerError::Invalid(e.to_string()))?;
            if let Some(r) = x.receipt {
                rs.push(r)
            }
            if let Some(e) = x.event {
                es.push(e)
            }
        }
        Ok((p, rs, es))
    }
    pub(super) fn project(&self) -> Result<(), LedgerError> {
        let (mut p, rs, es) = self.read_state()?;
        p.receipts = rs;
        p.events = es;
        self.write_projection(&p)?;
        let mut rows: Vec<(String, String)> = p
            .receipts
            .iter()
            .map(|r| {
                (
                    r.document_date.clone().unwrap_or_else(|| "日期未知".into()),
                    format!(
                        "- 文件日期={}｜收件時間={}｜檔案={}｜文件識別={}｜角色={}｜狀態={:?}｜收據={}｜路徑={}\n",
                        r.document_date.as_deref().unwrap_or("日期未知"),
                        r.observed_utc,
                        r.path.rsplit('/').next().unwrap_or(&r.path),
                        r.document_id.as_deref().unwrap_or("未知"),
                        r.role,
                        r.stage,
                        r.id,
                        r.path
                    ),
                )
            })
            .collect();
        rows.extend(p.events.iter().map(|e| {
            (
                e.event_date.clone().unwrap_or_else(|| "日期未知".into()),
                format!(
                    "- 事件日期={}｜事件={}｜收據={}｜觀察時間={}{}{}{}{}\n",
                    e.event_date.as_deref().unwrap_or("日期未知"),
                    e.kind,
                    e.receipt_id,
                    e.observed_utc,
                    e.note
                        .as_deref()
                        .map(|n| format!(" | {n}"))
                        .unwrap_or_default(),
                    e.new_owner_project
                        .as_deref()
                        .map(|owner| format!(" | 新歸屬={owner}"))
                        .unwrap_or_default(),
                    e.new_relation
                        .as_deref()
                        .map(|relation| format!(" | 新關係={relation}"))
                        .unwrap_or_default(),
                    e.provenance
                        .as_deref()
                        .map(|provenance| format!(" | 更正依據={provenance}"))
                        .unwrap_or_default()
                ),
            )
        }));
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let body = rows.into_iter().map(|x| x.1).collect::<String>();
        let t = self.root.join(TIMELINE);
        let old = fs::read_to_string(&t).unwrap_or_default();
        let keep = if let (Some(a), Some(b)) = (old.find(BEGIN), old.find(END)) {
            format!("{}{}", &old[..a], &old[b + END.len()..])
        } else {
            old
        };
        atomic(
            &t,
            format!("{}{}\n# {}\n{}{}\n", keep, BEGIN, p.title, body, END).as_bytes(),
        )
    }
    pub(super) fn write_projection(&self, p: &Projection) -> Result<(), LedgerError> {
        atomic(
            &self.index_path,
            &serde_json::to_vec_pretty(p).map_err(|e| LedgerError::Invalid(e.to_string()))?,
        )
    }
}
