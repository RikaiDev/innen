//! The immutable decision-event contract and the validation that keeps the
//! event graph well formed before anything selects from it.

use crate::ids::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_DECISIONS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub stable_prefix: Value,
    pub task: Task,
    pub budget_tokens: usize,
    /// Immutable decision events in ascending canonical UTC `observed_at` order.
    pub events: Vec<Decision>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub project: String,
    pub action: String,
    #[serde(default)]
    pub state: TaskState,
    #[serde(default)]
    pub conditions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Active,
    Paused,
    Completed,
}

impl Default for TaskState {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub id: String,
    pub project: String,
    pub task_id: String,
    pub actions: Vec<String>,
    pub statement: String,
    pub rationale: String,
    pub conditions: BTreeMap<String, String>,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub lessons: Vec<String>,
    #[serde(default)]
    pub reopen_when: Vec<String>,
    /// Canonical UTC timestamp: `YYYY-MM-DDTHH:MM:SSZ`.
    pub observed_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PreparedHistory {
    pub resolution: &'static str,
    pub history_source_sha256: String,
    pub selected_decision_ids: Vec<String>,
    pub guarded_decision_ids: Vec<String>,
    pub competing_decision_ids: Vec<String>,
    /// Full original events for this task's selected causal scope only.
    /// This is a reference-token comparison, not API billing.
    pub history_before_reference_tokens: usize,
    pub history_after_reference_tokens: usize,
    pub prepared: crate::middleware::Prepared,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("history request has {actual} decisions, exceeding the {max} decision bound")]
    TooManyDecisions { actual: usize, max: usize },
    #[error("history task project and action must be nonempty")]
    InvalidTask,
    #[error("history task `{task_id}` is {state} and cannot prepare an active packet")]
    InactiveTask {
        task_id: String,
        state: &'static str,
    },
    #[error("no current history decision matches project `{project}` and action `{action}`")]
    NoMatchingDecision { project: String, action: String },
    #[error("history decision at index {index} has an invalid immutable shape")]
    InvalidDecision { index: usize },
    #[error("history decision `{id}` duplicates an earlier id")]
    DuplicateId { id: String },
    #[error("history decision `{id}` has invalid observed_at `{observed_at}`")]
    InvalidObservedAt { id: String, observed_at: String },
    #[error("history decision `{id}` is not in chronological order")]
    NonChronological { id: String },
    #[error("history decision `{id}` references missing or later decision `{reference}`")]
    InvalidReference { id: String, reference: String },
    #[error("history decision `{id}` has a cross-project {kind} reference `{reference}`")]
    CrossProjectReference {
        id: String,
        kind: &'static str,
        reference: String,
    },
    #[error("history decision `{id}` has a cross-task {kind} reference `{reference}`")]
    CrossTaskReference {
        id: String,
        kind: &'static str,
        reference: String,
    },
    #[error("history source hash does not match the supplied request")]
    SourceMismatch { expected: String, actual: String },
    #[error("requested history decision ids are unknown: {ids:?}")]
    UnknownDecisions { ids: Vec<String> },
    #[error(transparent)]
    Middleware(#[from] crate::middleware::Error),
}

pub fn source_sha256(request: &Request) -> String {
    sha256_hex(&serde_json::to_vec(request).expect("history request serializes"))
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .any(|index| !bytes[*index].is_ascii_digit())
    {
        return false;
    }
    let number = |start| {
        std::str::from_utf8(&bytes[start..start + 2])
            .ok()?
            .parse::<u32>()
            .ok()
    };
    let year = std::str::from_utf8(&bytes[0..4])
        .ok()
        .and_then(|x| x.parse::<u32>().ok());
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        year,
        number(5),
        number(8),
        number(11),
        number(14),
        number(17),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day >= 1 && day <= max_day && hour < 24 && minute < 60 && second < 60
}

fn valid_source(source: &Source) -> bool {
    !source.id.trim().is_empty()
        && source.sha256.len() == 64
        && source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn present(value: &str) -> bool {
    !value.trim().is_empty()
}

pub(crate) fn validate(request: &Request) -> Result<HashMap<&str, usize>, Error> {
    if request.events.len() > MAX_DECISIONS {
        return Err(Error::TooManyDecisions {
            actual: request.events.len(),
            max: MAX_DECISIONS,
        });
    }
    if !present(&request.task.id)
        || !present(&request.task.project)
        || !present(&request.task.action)
    {
        return Err(Error::InvalidTask);
    }
    let mut positions = HashMap::new();
    let mut previous_time: Option<&str> = None;
    for (index, decision) in request.events.iter().enumerate() {
        if !present(&decision.id)
            || !present(&decision.project)
            || !present(&decision.task_id)
            || decision.actions.is_empty()
            || decision.actions.iter().any(|action| !present(action))
            || !present(&decision.statement)
            || !present(&decision.rationale)
            || decision.sources.is_empty()
            || decision.sources.iter().any(|source| !valid_source(source))
        {
            return Err(Error::InvalidDecision { index });
        }
        if !valid_timestamp(&decision.observed_at) {
            return Err(Error::InvalidObservedAt {
                id: decision.id.clone(),
                observed_at: decision.observed_at.clone(),
            });
        }
        if previous_time.is_some_and(|previous| previous > decision.observed_at.as_str()) {
            return Err(Error::NonChronological {
                id: decision.id.clone(),
            });
        }
        previous_time = Some(&decision.observed_at);
        if positions.insert(decision.id.as_str(), index).is_some() {
            return Err(Error::DuplicateId {
                id: decision.id.clone(),
            });
        }
    }
    for (index, decision) in request.events.iter().enumerate() {
        for (kind, refs) in [
            ("supersession", &decision.supersedes),
            ("dependency", &decision.depends_on),
        ] {
            let mut unique = HashSet::new();
            for reference in refs {
                if !present(reference) || !unique.insert(reference.as_str()) {
                    return Err(Error::InvalidDecision { index });
                }
                let Some(&other_index) = positions.get(reference.as_str()) else {
                    return Err(Error::InvalidReference {
                        id: decision.id.clone(),
                        reference: reference.clone(),
                    });
                };
                if other_index >= index {
                    return Err(Error::InvalidReference {
                        id: decision.id.clone(),
                        reference: reference.clone(),
                    });
                }
                if request.events[other_index].project != decision.project {
                    return Err(Error::CrossProjectReference {
                        id: decision.id.clone(),
                        kind,
                        reference: reference.clone(),
                    });
                }
                if request.events[other_index].task_id != decision.task_id {
                    return Err(Error::CrossTaskReference {
                        id: decision.id.clone(),
                        kind,
                        reference: reference.clone(),
                    });
                }
            }
        }
    }
    Ok(positions)
}
