use crate::{Result, StateError, Workspace, WorkspaceKind, WorkspaceStore};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyRepo {
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LegacyState {
    #[serde(default)]
    pub repos: Vec<LegacyRepo>,
}

pub fn migrate_from_json(json_path: &Path, store: &dyn WorkspaceStore) -> Result<MigrationResult> {
    if !json_path.exists() {
        return Ok(MigrationResult::NoLegacyState);
    }

    let body = std::fs::read_to_string(json_path).map_err(|source| StateError::Io {
        path: json_path.to_path_buf(),
        source,
    })?;

    let legacy: LegacyState = serde_json::from_str(&body).map_err(|source| StateError::Parse {
        path: json_path.to_path_buf(),
        source,
    })?;

    if legacy.repos.is_empty() {
        return Ok(MigrationResult::EmptyState);
    }

    let mut migrated = 0;
    for repo in legacy.repos {
        let workspace = Workspace {
            id: uuid::Uuid::new_v4().to_string(),
            kind: WorkspaceKind::Folder,
            path: repo.path.to_str().unwrap_or_default().to_string(),
            label: None,
            position: migrated,
        };

        store.save_workspace(workspace)?;
        migrated += 1;
    }

    backup_legacy_file(json_path)?;

    Ok(MigrationResult::Migrated { count: migrated })
}

fn backup_legacy_file(path: &Path) -> Result<()> {
    let backup_path = format!(
        "{}.backup-{}",
        path.display(),
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );

    std::fs::rename(path, &backup_path).map_err(|source| StateError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

#[derive(Debug)]
pub enum MigrationResult {
    NoLegacyState,
    EmptyState,
    Migrated { count: u32 },
}

impl std::fmt::Display for MigrationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLegacyState => write!(f, "No legacy state.json found"),
            Self::EmptyState => write!(f, "Legacy state.json was empty"),
            Self::Migrated { count } => write!(f, "Migrated {} workspaces to FeltDB", count),
        }
    }
}
