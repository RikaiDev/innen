//! Append-only project archive receipts. Source bytes are never copied.
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const TIMELINE: &str = "專案時間線.md";
const INDEX: &str = ".index.json";
const LEGACY_INDEX: &str = "檔案索引.json";
const EVENTS: &str = ".innen-ledger/events.jsonl";
const LOCK: &str = ".innen-ledger/ledger.lock";
const MANIFEST: &str = ".innen-ledger/manifest.json";
const BEGIN: &str = "<!-- innen-ledger:start -->";
const END: &str = "<!-- innen-ledger:end -->";
const MAGIC: &str = "innen-project-ledger-v1";
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("io: {0}")]
    Io(String),
    #[error("invalid ledger: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("downloads is intake-only: {0}")]
    Downloads(String),
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("ledger event appended but projection failed: {0}")]
    ProjectionAfterAppend(String),
}
#[derive(Debug, Clone)]
pub struct InitOptions {
    pub root: PathBuf,
    pub project_id: String,
    pub title: String,
    pub downloads_root: Option<PathBuf>,
    pub categories: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleStage {
    Intake,
    Authoring,
    Verification,
    Delivery,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Document,
    Event,
    Dataset,
    Other,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub document_id: Option<String>,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub source_kind: SourceKind,
    pub role: String,
    pub stage: LifecycleStage,
    pub owner_project: String,
    pub relation: String,
    pub observed_utc: String,
    pub document_date: Option<String>,
    pub authority: Option<String>,
    pub evidence_status: Option<String>,
    pub provenance: Option<String>,
    pub approval_valid_from: Option<String>,
    pub approval_valid_until: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    #[serde(default)]
    pub sequence: u64,
    pub kind: String,
    pub receipt_id: String,
    pub observed_utc: String,
    pub event_date: Option<String>,
    pub supersedes: Option<String>,
    pub new_path: Option<String>,
    pub expected_sha256: Option<String>,
    pub note: Option<String>,
    pub new_owner_project: Option<String>,
    pub new_relation: Option<String>,
    pub provenance: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEvent {
    magic: String,
    receipt: Option<Receipt>,
    event: Option<EventRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Projection {
    magic: String,
    pub project_id: String,
    pub title: String,
    pub categories: Vec<String>,
    pub receipts: Vec<Receipt>,
    pub events: Vec<EventRecord>,
}
#[derive(Debug, Clone)]
pub struct AddOptions {
    pub path: PathBuf,
    pub document_id: Option<String>,
    pub source_kind: SourceKind,
    pub role: String,
    pub stage: LifecycleStage,
    pub owner_project: String,
    pub relation: String,
    pub document_date: Option<String>,
    pub authority: Option<String>,
    pub evidence_status: Option<String>,
    pub provenance: Option<String>,
    pub approval_valid_from: Option<String>,
    pub approval_valid_until: Option<String>,
    pub downloads_root: Option<PathBuf>,
}
#[derive(Debug, Clone)]
pub enum LedgerEvent {
    Note {
        receipt_id: String,
        note: String,
        event_date: Option<String>,
    },
    Supersede {
        receipt_id: String,
        superseded_receipt_id: String,
        event_date: Option<String>,
    },
    Relocate {
        receipt_id: String,
        new_path: PathBuf,
        expected_sha256: String,
        event_date: Option<String>,
    },
    Reclassify {
        receipt_id: String,
        new_owner_project: Option<String>,
        new_relation: String,
        provenance: String,
        event_date: Option<String>,
    },
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    pub active: usize,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
    pub superseded: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct Ledger {
    root: PathBuf,
    downloads_root: PathBuf,
    index_path: PathBuf,
}
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
    fn append_locked(&self, x: FileEvent) -> Result<(), LedgerError> {
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
    fn read_state(&self) -> Result<(Projection, Vec<Receipt>, Vec<EventRecord>), LedgerError> {
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
    fn project(&self) -> Result<(), LedgerError> {
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
    fn write_projection(&self, p: &Projection) -> Result<(), LedgerError> {
        atomic(
            &self.index_path,
            &serde_json::to_vec_pretty(p).map_err(|e| LedgerError::Invalid(e.to_string()))?,
        )
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
fn hash_file(p: &Path) -> Result<(String, u64), std::io::Error> {
    let mut f = File::open(p)?;
    let mut h = sha2::Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    let mut n = 0;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64
    }
    let d = h.finalize();
    let s = d.iter().map(|b| format!("{b:02x}")).collect();
    Ok((s, n))
}
fn lock(p: &Path) -> Result<File, LedgerError> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(p)
        .map_err(io)?;
    f.lock_exclusive().map_err(io)?;
    Ok(f)
}
fn atomic(p: &Path, b: &[u8]) -> Result<(), LedgerError> {
    let t = p.with_file_name(format!(
        ".{}.tmp-{}",
        p.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&t)
        .map_err(io)?;
    f.write_all(b).map_err(io)?;
    f.sync_all().map_err(io)?;
    fs::rename(&t, p).map_err(io)
}
fn atomic_create(p: &Path, b: &[u8]) -> Result<(), LedgerError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .map_err(io)?;
    f.write_all(b).map_err(io)?;
    f.sync_all().map_err(io)
}
fn io(e: std::io::Error) -> LedgerError {
    LedgerError::Io(e.to_string())
}
fn absolute(p: &Path) -> Result<PathBuf, LedgerError> {
    if !p.is_absolute() {
        return Err(LedgerError::Invalid(format!(
            "path must be absolute: {}",
            p.display()
        )));
    }
    let mut tail = PathBuf::new();
    let mut cur = p;
    while !cur.exists() {
        let n = cur
            .parent()
            .ok_or_else(|| LedgerError::Invalid("cannot resolve path".into()))?;
        tail = cur
            .file_name()
            .map(|x| {
                let mut q = PathBuf::from(x);
                q.push(&tail);
                q
            })
            .unwrap_or(tail);
        cur = n
    }
    let mut out = fs::canonicalize(cur).map_err(io)?;
    if !tail.as_os_str().is_empty() {
        out.push(tail);
    }
    Ok(out)
}
fn under(p: &Path, b: &Path) -> bool {
    p == b || p.strip_prefix(b).is_ok()
}
fn default_downloads() -> Result<PathBuf, LedgerError> {
    absolute(
        &std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Downloads"),
    )
}
