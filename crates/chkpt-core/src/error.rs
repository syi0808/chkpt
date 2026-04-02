use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChkpttError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Bitcode error: {0}")]
    Bitcode(String),

    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("Lock held by another process")]
    LockHeld,

    #[error("Guardrail exceeded: {0}")]
    GuardrailExceeded(String),

    #[error("Store corrupted: {0}")]
    StoreCorrupted(String),

    #[error("Object not found: {0}")]
    ObjectNotFound(String),

    #[error("Restore failed: {0}")]
    RestoreFailed(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ChkpttError>;

impl From<bitcode::Error> for ChkpttError {
    fn from(e: bitcode::Error) -> Self {
        ChkpttError::Bitcode(e.to_string())
    }
}
