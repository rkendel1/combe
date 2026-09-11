use crate::{
    Pane, Tab, Workspace, WorkspaceState, WorkspaceStore, Result, StateError,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const STATE_VERSION: u32 = 1;
const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct FileState {
    version: u32,
    selected_workspace_id: Option<String>,
    workspaces: Vec<Workspace>,
    tabs: Vec<Tab>,
    panes: Vec<Pane>,
}

pub struct FileWorkspaceStore {
    state_path: PathBuf,
}

impl FileWorkspaceStore {
    pub fn new(state_path: PathBuf) -> Self {
        Self { state_path }
    }

    pub fn for_combe() -> Result<Self> {
        let state_dir = dirs::data_dir()
            .ok_or_else(|| StateError::FeltDbError("Cannot determine data directory".to_string()))?
            .join("combe");
        let state_path = state_dir.join("workspace_state.json");
        ensure_dir(&state_dir)?;
        Ok(Self::new(state_path))
    }

    fn load_file(&self) -> Result<FileState> {
        match fs::read_to_string(&self.state_path) {
            Ok(body) => {
                let state: FileState = serde_json::from_str(&body).map_err(|source| {
                    StateError::Parse {
                        path: self.state_path.clone(),
                        source,
                    }
                })?;
                if state.version != STATE_VERSION {
                    return Err(StateError::MigrationError(format!(
                        "Unsupported state version: {}",
                        state.version
                    )));
                }
                Ok(state)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(FileState {
                    version: STATE_VERSION,
                    selected_workspace_id: None,
                    workspaces: Vec::new(),
                    tabs: Vec::new(),
                    panes: Vec::new(),
                })
            }
            Err(source) => Err(StateError::Io {
                path: self.state_path.clone(),
                source,
            }),
        }
    }

    fn save_file(&self, state: &FileState) -> Result<()> {
        let body = serde_json::to_string_pretty(state)?;

        if body.len() > MAX_MESSAGE_SIZE {
            return Err(StateError::FeltDbError(format!(
                "State too large: {} bytes (max {})",
                body.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        let parent = self
            .state_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));

        let write = || -> std::io::Result<()> {
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            temp.write_all(body.as_bytes())?;
            temp.as_file().sync_all()?;
            temp.persist(&self.state_path).map_err(|err| err.error)?;
            Ok(())
        };

        write().map_err(|source| StateError::Io {
            path: self.state_path.clone(),
            source,
        })
    }
}

impl WorkspaceStore for FileWorkspaceStore {
    fn load_state(&self) -> Result<WorkspaceState> {
        let file_state = self.load_file()?;
        Ok(WorkspaceState {
            selected_workspace_id: file_state.selected_workspace_id,
            workspaces: file_state.workspaces,
        })
    }

    fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        let mut file_state = self.load_file()?;
        let index = file_state
            .workspaces
            .iter()
            .position(|w| w.id == workspace.id);
        match index {
            Some(i) => file_state.workspaces[i] = workspace,
            None => file_state.workspaces.push(workspace),
        }
        self.save_file(&file_state)
    }

    fn save_tab(&self, tab: Tab) -> Result<()> {
        let mut file_state = self.load_file()?;
        let index = file_state.tabs.iter().position(|t| t.id == tab.id);
        match index {
            Some(i) => file_state.tabs[i] = tab,
            None => file_state.tabs.push(tab),
        }
        self.save_file(&file_state)
    }

    fn save_pane(&self, pane: Pane) -> Result<()> {
        let mut file_state = self.load_file()?;
        let index = file_state.panes.iter().position(|p| p.id == pane.id);
        match index {
            Some(i) => file_state.panes[i] = pane,
            None => file_state.panes.push(pane),
        }
        self.save_file(&file_state)
    }

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        let file_state = self.load_file()?;
        Ok(file_state.workspaces.into_iter().find(|w| w.id == id))
    }

    fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        let file_state = self.load_file()?;
        Ok(file_state.tabs.into_iter().find(|t| t.id == id))
    }

    fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        let file_state = self.load_file()?;
        Ok(file_state.panes.into_iter().find(|p| p.id == id))
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let file_state = self.load_file()?;
        Ok(file_state.workspaces)
    }

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        let file_state = self.load_file()?;
        Ok(file_state
            .tabs
            .into_iter()
            .filter(|t| t.workspace_id == workspace_id)
            .collect())
    }

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        let file_state = self.load_file()?;
        Ok(file_state
            .panes
            .into_iter()
            .filter(|p| p.tab_id == tab_id)
            .collect())
    }

    fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        let mut file_state = self.load_file()?;
        let affected_tab_ids: Vec<_> = file_state
            .tabs
            .iter()
            .filter(|t| t.workspace_id == workspace_id)
            .map(|t| t.id.clone())
            .collect();
        file_state.workspaces.retain(|w| w.id != workspace_id);
        file_state.tabs.retain(|t| t.workspace_id != workspace_id);
        file_state
            .panes
            .retain(|p| !affected_tab_ids.contains(&p.tab_id));

        self.save_file(&file_state)
    }

    fn delete_tab(&self, tab_id: &str) -> Result<()> {
        let mut file_state = self.load_file()?;
        file_state.tabs.retain(|t| t.id != tab_id);
        file_state.panes.retain(|p| p.tab_id != tab_id);
        self.save_file(&file_state)
    }

    fn delete_pane(&self, pane_id: &str) -> Result<()> {
        let mut file_state = self.load_file()?;
        file_state.panes.retain(|p| p.id != pane_id);
        self.save_file(&file_state)
    }

    fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()> {
        let mut file_state = self.load_file()?;
        file_state.selected_workspace_id = workspace_id;
        self.save_file(&file_state)
    }
}

fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| StateError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkspaceKind;

    #[test]
    fn test_new_store_creates_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileWorkspaceStore::new(dir.path().join("state.json"));
        let state = store.load_state().unwrap();
        assert!(state.workspaces.is_empty());
        assert!(state.selected_workspace_id.is_none());
    }

    #[test]
    fn test_save_and_load_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileWorkspaceStore::new(dir.path().join("state.json"));

        let workspace = Workspace::new(WorkspaceKind::Folder, "/tmp/test".to_string());
        let ws_id = workspace.id.clone();
        store.save_workspace(workspace).unwrap();

        let loaded = store.load_workspace(&ws_id).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().id, ws_id);
    }

    #[test]
    fn test_save_and_load_tab() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileWorkspaceStore::new(dir.path().join("state.json"));

        let workspace = Workspace::new(WorkspaceKind::Folder, "/tmp/test".to_string());
        let ws_id = workspace.id.clone();
        store.save_workspace(workspace).unwrap();

        let tab = Tab::new(ws_id.clone(), "Terminal".to_string());
        let tab_id = tab.id.clone();
        store.save_tab(tab).unwrap();

        let loaded = store.load_tab(&tab_id).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().workspace_id, ws_id);
    }

    #[test]
    fn test_cascade_delete_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileWorkspaceStore::new(dir.path().join("state.json"));

        let workspace = Workspace::new(WorkspaceKind::Folder, "/tmp/test".to_string());
        let ws_id = workspace.id.clone();
        store.save_workspace(workspace).unwrap();

        let tab = Tab::new(ws_id.clone(), "Terminal".to_string());
        let tab_id = tab.id.clone();
        store.save_tab(tab).unwrap();

        let pane = Pane::new(tab_id.clone(), "/tmp".to_string());
        let pane_id = pane.id.clone();
        store.save_pane(pane).unwrap();

        store.delete_workspace(&ws_id).unwrap();

        assert!(store.load_workspace(&ws_id).unwrap().is_none());
        assert!(store.load_tab(&tab_id).unwrap().is_none());
        assert!(store.load_pane(&pane_id).unwrap().is_none());
    }
}
