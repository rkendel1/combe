use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
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

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("entity not found: {entity_type} {id}")]
    NotFound {
        entity_type: String,
        id: String,
    },

    #[error("invalid entity: {0}")]
    InvalidEntity(String),

    #[error("feltdb error: {0}")]
    FeltDbError(String),

    #[error("migration error: {0}")]
    MigrationError(String),
}

pub type Result<T> = std::result::Result<T, StateError>;
