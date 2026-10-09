//! Append-only project archive receipts. Source bytes are never copied.
mod fs;
mod model;
mod ops;
mod state;

pub use model::{
    AddOptions, CheckReport, EventRecord, InitOptions, Ledger, LedgerError, LedgerEvent,
    LifecycleStage, Receipt, SourceKind,
};
