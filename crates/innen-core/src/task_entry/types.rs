//! Request and result types for task-context retrieval.

/// Options for task-context retrieval.
#[derive(Debug, Clone)]
pub struct TaskEntryOptions {
    pub q: String,
    pub as_of: Option<String>,
    pub limit: usize,
    pub offset: usize,
    pub include_expired: bool,
    pub view: String, // "context" (default compact brief) or "evidence"
}

impl Default for TaskEntryOptions {
    fn default() -> Self {
        Self {
            q: String::new(),
            as_of: None,
            limit: 20,
            offset: 0,
            include_expired: false,
            view: "context".to_string(),
        }
    }
}
