use combe_state::{FileWorkspaceStore, Pane, Tab, Workspace, WorkspaceKind, WorkspaceState, WorkspaceStore, Result};
use std::sync::{Arc, Mutex};

pub struct SessionManager {
    store: Arc<Mutex<Box<dyn WorkspaceStore>>>,
}

impl SessionManager {
    pub fn new() -> Result<Self> {
        let store = Box::new(FileWorkspaceStore::for_combe()?);
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
        })
    }

    pub fn load_state(&self) -> Result<WorkspaceState> {
        let store = self.store.lock().unwrap();
        store.load_state()
    }

    pub fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.save_workspace(workspace)
    }

    pub fn save_tab(&self, tab: Tab) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.save_tab(tab)
    }

    pub fn save_pane(&self, pane: Pane) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.save_pane(pane)
    }

    pub fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        let store = self.store.lock().unwrap();
        store.load_workspace(id)
    }

    pub fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        let store = self.store.lock().unwrap();
        store.load_tab(id)
    }

    pub fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        let store = self.store.lock().unwrap();
        store.load_pane(id)
    }

    pub fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let store = self.store.lock().unwrap();
        store.list_workspaces()
    }

    pub fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        let store = self.store.lock().unwrap();
        store.list_tabs(workspace_id)
    }

    pub fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        let store = self.store.lock().unwrap();
        store.list_panes(tab_id)
    }

    pub fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.delete_workspace(workspace_id)
    }

    pub fn delete_tab(&self, tab_id: &str) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.delete_tab(tab_id)
    }

    pub fn delete_pane(&self, pane_id: &str) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.delete_pane(pane_id)
    }

    pub fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()> {
        let store = self.store.lock().unwrap();
        store.set_selected_workspace(workspace_id)
    }
}

pub fn should_skip_restore() -> bool {
    std::env::args().any(|arg| arg == "--fresh")
}
