use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid state file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("entity not found: {entity_type} {id}")]
    NotFound {
        entity_type: String,
        id: String,
    },

    #[error("invalid entity: {0}")]
    InvalidEntity(String),

    #[error("database schema mismatch: expected version {expected}, got {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },

    #[error("transaction error: {0}")]
    Transaction(String),
}

pub type Result<T> = std::result::Result<T, StateError>;
