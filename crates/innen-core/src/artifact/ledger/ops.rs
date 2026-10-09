//! Public ledger operations, and the receipt derivation they read through.

use std::fs::{self, File};
use std::path::Path;

use super::fs::{absolute, atomic_create, default_downloads, hash_file, io, lock, under};
use super::model::{
    AddOptions, CheckReport, EventRecord, FileEvent, InitOptions, Ledger, LedgerError, LedgerEvent,
    LifecycleStage, Projection, Receipt, EVENTS, INDEX, LEGACY_INDEX, LOCK, MAGIC, MANIFEST,
    TIMELINE,
};

impl Ledger {
    pub fn init(o: InitOptions) -> Result<Self, LedgerError> {
        if o.title.is_empty() || o.project_id.is_empty() {
            return Err(LedgerError::Invalid(
                "title and project_id are required".into(),
            ));
        }
        let root = absolute(&o.root)?;
        if root.file_name().and_then(|x| x.to_str()) != Some(o.title.as_str()) {
            return Err(LedgerError::Invalid(
                "archive basename must equal exact title".into(),
            ));
        }
        let dl = o
            .downloads_root
            .map(|p| absolute(&p))
            .transpose()?
            .unwrap_or(default_downloads()?);
        if under(&root, &dl) {
            return Err(LedgerError::Downloads(root.display().to_string()));
        }
        fs::create_dir_all(root.join(".innen-ledger")).map_err(io)?;
        let _l = lock(&root.join(LOCK))?;
        let manifest = root.join(MANIFEST);
        if manifest.exists() {
            let p: Projection = serde_json::from_slice(&fs::read(&manifest).map_err(io)?)
                .map_err(|e| LedgerError::Invalid(e.to_string()))?;
            if p.magic != MAGIC {
                return Err(LedgerError::Invalid(
                    "ledger manifest magic mismatch".into(),
                ));
            }
            if p.project_id != o.project_id || p.title != o.title {
                return Err(LedgerError::Conflict(
                    "existing ledger identity differs".into(),
                ));
            }
            let index_path = if root.join(INDEX).exists() {
                root.join(INDEX)
            } else if root.join(LEGACY_INDEX).exists() {
                root.join(LEGACY_INDEX)
            } else {
                root.join(INDEX)
            };
            return Ok(Self {
                root,
                downloads_root: dl,
                index_path,
            });
        }
        if root.join(TIMELINE).exists()
            || root.join(INDEX).exists()
            || root.join(LEGACY_INDEX).exists()
            || root.join(EVENTS).exists()
        {
            return Err(LedgerError::Conflict(
                "preexisting timeline/index is unrelated".into(),
            ));
        }
        let l = Self {
            root: root.clone(),
            downloads_root: dl,
            index_path: root.join(INDEX),
        };
        let p = Projection {
            magic: MAGIC.into(),
            project_id: o.project_id,
            title: o.title,
            categories: o.categories,
            receipts: vec![],
            events: vec![],
        };
        atomic_create(
            &manifest,
            &serde_json::to_vec_pretty(&p).map_err(|e| LedgerError::Invalid(e.to_string()))?,
        )?;
        File::create(l.root.join(EVENTS))
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
        l.project()?;
        Ok(l)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn add(&self, o: AddOptions) -> Result<Receipt, LedgerError> {
        if o.relation == "belongs_to" && o.owner_project.trim().is_empty() {
            return Err(LedgerError::Invalid("belongs_to owner is required".into()));
        }
        if o.role.trim().is_empty() || o.provenance.as_deref().unwrap_or("").trim().is_empty() {
            return Err(LedgerError::Invalid(
                "role and provenance are required".into(),
            ));
        }
        if !matches!(o.relation.as_str(), "belongs_to" | "reference") {
            return Err(LedgerError::Invalid(
                "relation must be belongs_to or reference".into(),
            ));
        }
        let p = absolute(&o.path)?;
        let dl = o
            .downloads_root
            .map(|x| absolute(&x))
            .transpose()?
            .unwrap_or_else(|| self.downloads_root.clone());
        if under(&p, &dl) && o.stage != LifecycleStage::Intake {
            return Err(LedgerError::Downloads(p.display().to_string()));
        }
        let (ident, _, _) = self.read_state()?;
        if o.relation == "belongs_to" && o.owner_project != ident.project_id {
            return Err(LedgerError::Invalid(
                "belongs_to owner must match archive project".into(),
            ));
        }
        let (sha, bytes) = hash_file(&p).map_err(io)?;
        let _l = lock(&self.root.join(LOCK))?;
        let (_, old, _) = self.read_state()?;
        let archive_key =
            crate::ids::sha256_hex(format!("{}\0{}", ident.project_id, ident.title).as_bytes());
        let r = Receipt {
            id: format!("receipt:{}:{sha}:{}", &archive_key[..16], old.len() + 1),
            document_id: o.document_id,
            path: p.display().to_string(),
            sha256: sha,
            bytes,
            source_kind: o.source_kind,
            role: o.role,
            stage: o.stage,
            owner_project: o.owner_project,
            relation: o.relation,
            observed_utc: crate::journal::observed_utc_now(),
            document_date: o.document_date,
            authority: o.authority,
            evidence_status: o.evidence_status,
            provenance: o.provenance,
            approval_valid_from: o.approval_valid_from,
            approval_valid_until: o.approval_valid_until,
        };
        self.append_locked(FileEvent {
            magic: MAGIC.into(),
            receipt: Some(r.clone()),
            event: None,
        })?;
        Ok(r)
    }
    pub fn event(&self, x: LedgerEvent) -> Result<EventRecord, LedgerError> {
        let _guard = lock(&self.root.join(LOCK))?;
        let (ident, rs, es) = self.read_state()?;
        let e = match x {
            LedgerEvent::Note {
                receipt_id,
                note,
                event_date,
            } => {
                require(&rs, &receipt_id)?;
                EventRecord {
                    sequence: 0,
                    kind: "note".into(),
                    receipt_id,
                    observed_utc: crate::journal::observed_utc_now(),
                    event_date,
                    supersedes: None,
                    new_path: None,
                    expected_sha256: None,
                    note: Some(note),
                    new_owner_project: None,
                    new_relation: None,
                    provenance: None,
                }
            }
            LedgerEvent::Supersede {
                receipt_id,
                superseded_receipt_id,
                event_date,
            } => {
                let a = require(&rs, &receipt_id)?;
                let b = require(&rs, &superseded_receipt_id)?;
                if receipt_id == superseded_receipt_id
                    || a.owner_project != b.owner_project
                    || a.owner_project != ident.project_id
                {
                    return Err(LedgerError::Invalid(
                        "invalid self or cross-owner supersession".into(),
                    ));
                }
                EventRecord {
                    sequence: 0,
                    kind: "supersede".into(),
                    receipt_id,
                    observed_utc: crate::journal::observed_utc_now(),
                    event_date,
                    supersedes: Some(superseded_receipt_id),
                    new_path: None,
                    expected_sha256: None,
                    note: None,
                    new_owner_project: None,
                    new_relation: None,
                    provenance: None,
                }
            }
            LedgerEvent::Relocate {
                receipt_id,
                new_path,
                expected_sha256,
                event_date,
            } => {
                let r = require(&rs, &receipt_id)?;
                let p = absolute(&new_path)?;
                if under(&p, &self.downloads_root) && r.stage != LifecycleStage::Intake {
                    return Err(LedgerError::Downloads(p.display().to_string()));
                }
                let (actual, _) = hash_file(&p).map_err(io)?;
                if actual != expected_sha256 || actual != r.sha256 {
                    return Err(LedgerError::HashMismatch {
                        expected: expected_sha256,
                        actual,
                    });
                }
                EventRecord {
                    sequence: 0,
                    kind: "relocate".into(),
                    receipt_id,
                    observed_utc: crate::journal::observed_utc_now(),
                    event_date,
                    supersedes: None,
                    new_path: Some(p.display().to_string()),
                    expected_sha256: Some(actual),
                    note: None,
                    new_owner_project: None,
                    new_relation: None,
                    provenance: None,
                }
            }
            LedgerEvent::Reclassify {
                receipt_id,
                new_owner_project,
                new_relation,
                provenance,
                event_date,
            } => {
                require(&rs, &receipt_id)?;
                if provenance.trim().is_empty()
                    || !matches!(new_relation.as_str(), "belongs_to" | "reference")
                {
                    return Err(LedgerError::Invalid(
                        "reclassification requires provenance and a valid relation".into(),
                    ));
                }
                if new_relation == "belongs_to"
                    && new_owner_project.as_deref().unwrap_or("").trim().is_empty()
                {
                    return Err(LedgerError::Invalid(
                        "belongs_to reclassification requires owner".into(),
                    ));
                }
                if new_relation == "belongs_to"
                    && new_owner_project.as_deref() != Some(ident.project_id.as_str())
                {
                    return Err(LedgerError::Invalid(
                        "belongs_to reclassification owner must match archive project".into(),
                    ));
                }
                EventRecord {
                    sequence: 0,
                    kind: "reclassify".into(),
                    receipt_id,
                    observed_utc: crate::journal::observed_utc_now(),
                    event_date,
                    supersedes: None,
                    new_path: None,
                    expected_sha256: None,
                    note: None,
                    new_owner_project,
                    new_relation: Some(new_relation),
                    provenance: Some(provenance),
                }
            }
        };
        let mut e = e;
        e.sequence = es.len() as u64 + 1;
        self.append_locked(FileEvent {
            magic: MAGIC.into(),
            receipt: None,
            event: Some(e),
        })?;
        self.read_state()?
            .2
            .last()
            .cloned()
            .ok_or_else(|| LedgerError::Invalid("event append produced no event".into()))
    }
    pub fn receipt(&self, id: &str) -> Result<Receipt, LedgerError> {
        let (_, receipts, events) = self.read_state()?;
        Ok(effective(require(&receipts, id)?, &events))
    }
    pub fn check(&self) -> Result<CheckReport, LedgerError> {
        let (_, rs, es) = self.read_state()?;
        let mut out = CheckReport::default();
        for r in rs {
            if es
                .iter()
                .any(|e| e.kind == "supersede" && e.supersedes.as_deref() == Some(&r.id))
            {
                out.superseded.push(r.id);
                continue;
            }
            out.active += 1;
            match hash_file(Path::new(&current(&r, &es))) {
                Ok((s, _)) if s != r.sha256 => out.changed.push(r.id),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => out.missing.push(r.id),
                Err(e) => return Err(io(e)),
            }
        }
        Ok(out)
    }
}
fn require<'a>(rs: &'a [Receipt], id: &str) -> Result<&'a Receipt, LedgerError> {
    rs.iter()
        .find(|r| r.id == id)
        .ok_or_else(|| LedgerError::Invalid(format!("unknown receipt {id}")))
}
fn effective(receipt: &Receipt, events: &[EventRecord]) -> Receipt {
    let mut current = receipt.clone();
    for event in events.iter().filter(|event| event.receipt_id == receipt.id) {
        match event.kind.as_str() {
            "relocate" => {
                if let Some(path) = &event.new_path {
                    current.path = path.clone();
                }
            }
            "reclassify" => {
                if let Some(owner) = &event.new_owner_project {
                    current.owner_project = owner.clone();
                }
                if let Some(relation) = &event.new_relation {
                    current.relation = relation.clone();
                }
                if let Some(provenance) = &event.provenance {
                    current.provenance = Some(provenance.clone());
                }
            }
            _ => {}
        }
    }
    current
}
fn current(r: &Receipt, es: &[EventRecord]) -> String {
    es.iter()
        .filter(|e| e.kind == "relocate" && e.receipt_id == r.id)
        .next_back()
        .and_then(|e| e.new_path.clone())
        .unwrap_or_else(|| r.path.clone())
}
