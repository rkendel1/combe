use crate::{Pane, Result, StateError, Tab, Workspace, WorkspaceState, WorkspaceKind};
use crate::store::WorkspaceStore;
use std::path::PathBuf;

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

/// Session metadata stored in FeltDB.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct SessionMetadata {
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
            .query(|_: &SessionMetadata| true)
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
        let key = format!("workspace:{}", workspace.id);

        self.db
            .insert(&key, workspace)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }

    fn save_tab(&self, tab: Tab) -> Result<()> {
        let key = format!("tab:{}", tab.id);

        self.db
            .insert(&key, tab)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }

    fn save_pane(&self, pane: Pane) -> Result<()> {
        let key = format!("pane:{}", pane.id);

        self.db
            .insert(&key, pane)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        let id = id.to_string();
        let results = self
            .db
            .query(|workspace: &Workspace| workspace.id == id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().next())
    }

    fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        let id = id.to_string();
        let results = self
            .db
            .query(|tab: &Tab| tab.id == id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().next())
    }

    fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        let id = id.to_string();
        let results = self
            .db
            .query(|pane: &Pane| pane.id == id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results.into_iter().next())
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let results = self
            .db
            .query(|_: &Workspace| true)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results)
    }

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        let workspace_id = workspace_id.to_string();
        let results = self
            .db
            .query(move |tab: &Tab| tab.workspace_id == workspace_id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results)
    }

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        let tab_id = tab_id.to_string();
        let results = self
            .db
            .query(move |pane: &Pane| pane.tab_id == tab_id)
            .map_err(|e| StateError::FeltDbError(format!("Query failed: {}", e)))?;

        Ok(results)
    }

    fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        let workspace_key = format!("workspace:{}", workspace_id);

        // Get all tabs for this workspace
        let tabs = self.list_tabs(workspace_id)?;

        // Delete all panes for all tabs
        for tab in &tabs {
            let panes = self.list_panes(&tab.id)?;
            for pane in panes {
                self.delete_pane(&pane.id)?;
            }
        }

        // Delete all tabs
        for tab in tabs {
            let key = format!("tab:{}", tab.id);
            self.db
                .delete(&key)
                .map_err(|e| StateError::FeltDbError(format!("Delete failed: {}", e)))?;
        }

        // Delete the workspace
        self.db
            .delete(&workspace_key)
            .map_err(|e| StateError::FeltDbError(format!("Delete failed: {}", e)))?;

        Ok(())
    }

    fn delete_tab(&self, tab_id: &str) -> Result<()> {
        let tab_key = format!("tab:{}", tab_id);

        // Get all panes for this tab
        let panes = self.list_panes(tab_id)?;

        // Delete all panes
        for pane in panes {
            self.delete_pane(&pane.id)?;
        }

        // Delete the tab
        self.db
            .delete(&tab_key)
            .map_err(|e| StateError::FeltDbError(format!("Delete failed: {}", e)))?;

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
        let metadata = SessionMetadata { workspace_id };

        self.db
            .insert(key, metadata)
            .map_err(|e| StateError::FeltDbError(format!("Insert failed: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_store() -> Result<(FeltDbWorkspaceStore, TempDir)> {
        let temp_dir = TempDir::new()
            .map_err(|e| StateError::FeltDbError(format!("Failed to create temp dir: {}", e)))?;
        let db_path = temp_dir.path().join("test.db");

        let db = feltdb::FeltDb::open(&db_path)
            .map_err(|e| StateError::FeltDbError(format!("Failed to open FeltDB: {}", e)))?;

        Ok((FeltDbWorkspaceStore { db }, temp_dir))
    }

    #[test]
    fn test_create_and_load_workspace() -> Result<()> {
        let (store, _temp) = create_test_store()?;

        let ws = Workspace {
            id: "ws1".to_string(),
            kind: WorkspaceKind::Folder,
            path: "/home/user/project".to_string(),
            label: Some("My Project".to_string()),
            position: 0,
        };

        store.save_workspace(ws.clone())?;
        let loaded = store.load_workspace("ws1")?;

        assert_eq!(loaded, Some(ws));
        Ok(())
    }

    #[test]
    fn test_list_workspaces() -> Result<()> {
        let (store, _temp) = create_test_store()?;

        let ws1 = Workspace::new(WorkspaceKind::Folder, "/project1".to_string());
        let ws2 = Workspace::new(WorkspaceKind::Folder, "/project2".to_string());

        store.save_workspace(ws1.clone())?;
        store.save_workspace(ws2.clone())?;

        let workspaces = store.list_workspaces()?;
        assert_eq!(workspaces.len(), 2);
        Ok(())
    }

    #[test]
    fn test_workspace_tabs_and_panes() -> Result<()> {
        let (store, _temp) = create_test_store()?;

        let ws = Workspace::new(WorkspaceKind::Folder, "/project".to_string());
        store.save_workspace(ws.clone())?;

        let tab = Tab::new(ws.id.clone(), "Tab 1".to_string());
        store.save_tab(tab.clone())?;

        let pane = Pane::new(tab.id.clone(), "/project".to_string());
        store.save_pane(pane.clone())?;

        let loaded_tab = store.load_tab(&tab.id)?;
        assert_eq!(loaded_tab, Some(tab.clone()));

        let loaded_pane = store.load_pane(&pane.id)?;
        assert_eq!(loaded_pane, Some(pane));

        let tabs = store.list_tabs(&ws.id)?;
        assert_eq!(tabs.len(), 1);

        Ok(())
    }

    #[test]
    fn test_delete_workspace_cascade() -> Result<()> {
        let (store, _temp) = create_test_store()?;

        let ws = Workspace::new(WorkspaceKind::Folder, "/project".to_string());
        store.save_workspace(ws.clone())?;

        let tab = Tab::new(ws.id.clone(), "Tab 1".to_string());
        store.save_tab(tab.clone())?;

        let pane = Pane::new(tab.id.clone(), "/project".to_string());
        store.save_pane(pane.clone())?;

        store.delete_workspace(&ws.id)?;

        let loaded_ws = store.load_workspace(&ws.id)?;
        assert_eq!(loaded_ws, None);

        let tabs = store.list_tabs(&ws.id)?;
        assert_eq!(tabs.len(), 0);

        let loaded_pane = store.load_pane(&pane.id)?;
        assert_eq!(loaded_pane, None);

        Ok(())
    }

    #[test]
    fn test_selected_workspace() -> Result<()> {
        let (store, _temp) = create_test_store()?;

        let ws = Workspace::new(WorkspaceKind::Folder, "/project".to_string());
        store.save_workspace(ws.clone())?;

        store.set_selected_workspace(Some(ws.id.clone()))?;
        let state = store.load_state()?;

        assert_eq!(state.selected_workspace_id, Some(ws.id));
        Ok(())
    }

    #[test]
    fn test_persistence_across_reconnect() -> Result<()> {
        let temp_dir = TempDir::new()
            .map_err(|e| StateError::FeltDbError(format!("Failed to create temp dir: {}", e)))?;
        let db_path = temp_dir.path().join("persist.db");

        // Create and populate store
        {
            let db = feltdb::FeltDb::open(&db_path)
                .map_err(|e| StateError::FeltDbError(format!("Failed to open FeltDB: {}", e)))?;
            let store = FeltDbWorkspaceStore { db };

            let ws = Workspace::new(WorkspaceKind::Folder, "/project".to_string());
            store.save_workspace(ws)?;
        }

        // Reopen and verify
        {
            let db = feltdb::FeltDb::open(&db_path)
                .map_err(|e| StateError::FeltDbError(format!("Failed to open FeltDB: {}", e)))?;
            let store = FeltDbWorkspaceStore { db };

            let workspaces = store.list_workspaces()?;
            assert_eq!(workspaces.len(), 1);
        }

        Ok(())
    }
}
