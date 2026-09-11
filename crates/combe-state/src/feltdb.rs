use crate::{Pane, Result, StateError, Tab, Workspace, WorkspaceState};
use crate::store::WorkspaceStore;
use feltdb::AtomicMutation;
use std::path::PathBuf;
use uuid::Uuid;

/// FeltDbWorkspaceStore provides durable workspace state using the FeltDB Rust crate.
///
/// Uses FeltDB's direct Rust API for crash-safe, atomic persistence.
/// Database location: ~/Library/Application Support/combe/workspace-data (macOS)
///
/// Note on concurrency: FeltDB serializes all reads and writes behind a single mutex.
/// Combe's single native application process is the sole owner of this database.
/// Multiple processes accessing the same database simultaneously is not supported.
pub struct FeltDbWorkspaceStore {
    db: feltdb::FeltDb,
}

/// Wrapper types for storing records in FeltDB with proper serialization.
/// These wrap the Combe types to include metadata as needed.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct StoredWorkspace {
    #[serde(flatten)]
    workspace: Workspace,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct StoredTab {
    #[serde(flatten)]
    tab: Tab,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct StoredPane {
    #[serde(flatten)]
    pane: Pane,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct StoredSessionMetadata {
    workspace_id: Option<String>,
}

impl FeltDbWorkspaceStore {
    /// Create a new FeltDbWorkspaceStore using the default application support directory.
    pub fn new() -> Result<Self> {
        let db_path = Self::database_path()?;
        let db = feltdb::FeltDb::open(&db_path)
            .map_err(|e| StateError::FeltDbError(format!("Failed to open FeltDB: {}", e)))?;
        Ok(Self { db })
    }

    /// Get the default database path for Combe.
    fn database_path() -> Result<PathBuf> {
        let app_support = dirs::data_dir()
            .ok_or_else(|| StateError::FeltDbError("Cannot determine data directory".to_string()))?;
        Ok(app_support.join("combe").join("workspace-data"))
    }

    /// For CLI usage: get or create store with default path.
    pub fn for_combe() -> Result<Self> {
        Self::new()
    }
}

impl WorkspaceStore for FeltDbWorkspaceStore {
    fn load_state(&self) -> Result<WorkspaceState> {
        let workspaces = self.list_workspaces()?;

        // Load session metadata
        let selected_workspace_id = self
            .db
            .query(|_: &StoredSessionMetadata| true)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?
            .into_iter()
            .next()
            .and_then(|metadata| metadata.workspace_id);

        Ok(WorkspaceState {
            selected_workspace_id,
            workspaces,
        })
    }

    fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        let stored = StoredWorkspace { workspace };
        let key = format!("workspace:{}", stored.workspace.id);
        let value = serde_json::to_value(&stored)
            .map_err(|e| StateError::FeltDbError(format!("Serialization failed: {}", e)))?;

        self.db
            .insert(&key, value)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }

    fn save_tab(&self, tab: Tab) -> Result<()> {
        let stored = StoredTab { tab };
        let key = format!("tab:{}", stored.tab.id);
        let value = serde_json::to_value(&stored)
            .map_err(|e| StateError::FeltDbError(format!("Serialization failed: {}", e)))?;

        self.db
            .insert(&key, value)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }

    fn save_pane(&self, pane: Pane) -> Result<()> {
        let stored = StoredPane { pane };
        let key = format!("pane:{}", stored.pane.id);
        let value = serde_json::to_value(&stored)
            .map_err(|e| StateError::FeltDbError(format!("Serialization failed: {}", e)))?;

        self.db
            .insert(&key, value)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        let id = id.to_string();
        let results = self
            .db
            .query(|stored: &StoredWorkspace| stored.workspace.id == id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().next().map(|stored| stored.workspace))
    }

    fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        let id = id.to_string();
        let results = self
            .db
            .query(|stored: &StoredTab| stored.tab.id == id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().next().map(|stored| stored.tab))
    }

    fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        let id = id.to_string();
        let results = self
            .db
            .query(|stored: &StoredPane| stored.pane.id == id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().next().map(|stored| stored.pane))
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let results = self
            .db
            .query(|_: &StoredWorkspace| true)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().map(|stored| stored.workspace).collect())
    }

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        let workspace_id = workspace_id.to_string();
        let results = self
            .db
            .query(move |stored: &StoredTab| stored.tab.workspace_id == workspace_id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().map(|stored| stored.tab).collect())
    }

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        let tab_id = tab_id.to_string();
        let results = self
            .db
            .query(move |stored: &StoredPane| stored.pane.tab_id == tab_id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().map(|stored| stored.pane).collect())
    }

    fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        let workspace_key = format!("workspace:{}", workspace_id);

        // Get all tabs for this workspace
        let tabs = self.list_tabs(workspace_id)?;

        // Collect all pane keys to delete
        let mut pane_keys = Vec::new();
        for tab in &tabs {
            let panes = self.list_panes(&tab.id)?;
            for pane in panes {
                pane_keys.push(format!("pane:{}", pane.id));
            }
        }

        let tab_keys: Vec<String> = tabs.iter().map(|t| format!("tab:{}", t.id)).collect();

        // Build atomic transaction to delete all related records
        let mut mutations = Vec::new();
        mutations.push(AtomicMutation {
            capability: workspace_key.clone(),
            key: workspace_key,
            value: None,
        });

        for tab_key in tab_keys {
            mutations.push(AtomicMutation {
                capability: tab_key.clone(),
                key: tab_key,
                value: None,
            });
        }

        for pane_key in pane_keys {
            mutations.push(AtomicMutation {
                capability: pane_key.clone(),
                key: pane_key,
                value: None,
            });
        }

        let transaction_id = Uuid::new_v4().to_string();
        self.db
            .apply_atomic_transaction(&transaction_id, None, &[], &mutations, None)
            .map_err(|e| StateError::FeltDbError(format!("Transaction failed: {}", e)))?;

        Ok(())
    }

    fn delete_tab(&self, tab_id: &str) -> Result<()> {
        let tab_key = format!("tab:{}", tab_id);

        // Get all panes for this tab
        let panes = self.list_panes(tab_id)?;
        let pane_keys: Vec<String> = panes.iter().map(|p| format!("pane:{}", p.id)).collect();

        // Build atomic transaction to delete tab and all its panes
        let mut mutations = Vec::new();
        mutations.push(AtomicMutation {
            capability: tab_key.clone(),
            key: tab_key,
            value: None,
        });

        for pane_key in pane_keys {
            mutations.push(AtomicMutation {
                capability: pane_key.clone(),
                key: pane_key,
                value: None,
            });
        }

        let transaction_id = Uuid::new_v4().to_string();
        self.db
            .apply_atomic_transaction(&transaction_id, None, &[], &mutations, None)
            .map_err(|e| StateError::FeltDbError(format!("Transaction failed: {}", e)))?;

        Ok(())
    }

    fn delete_pane(&self, pane_id: &str) -> Result<()> {
        let key = format!("pane:{}", pane_id);
        self.db
            .delete(&key)
            .map_err(|e| StateError::FeltDbError(format!("Delete failed: {}", e)))?;
        Ok(())
    }

    fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()> {
        let key = "session:selected_workspace_id";
        let metadata = StoredSessionMetadata { workspace_id };
        let value = serde_json::to_value(&metadata)
            .map_err(|e| StateError::FeltDbError(format!("Serialization failed: {}", e)))?;

        self.db
            .insert(key, value)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }
}
