use crate::store::WorkspaceStore;
use crate::{Pane, Result, Tab, Workspace, WorkspaceState};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct InMemoryWorkspaceStore {
    workspaces: Arc<Mutex<HashMap<String, Workspace>>>,
    tabs: Arc<Mutex<HashMap<String, Tab>>>,
    panes: Arc<Mutex<HashMap<String, Pane>>>,
    selected_workspace: Arc<Mutex<Option<String>>>,
}

impl InMemoryWorkspaceStore {
    pub fn new() -> Self {
        Self {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            tabs: Arc::new(Mutex::new(HashMap::new())),
            panes: Arc::new(Mutex::new(HashMap::new())),
            selected_workspace: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for InMemoryWorkspaceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceStore for InMemoryWorkspaceStore {
    fn load_state(&self) -> Result<WorkspaceState> {
        let workspaces = self.workspaces.lock().unwrap().values().cloned().collect();
        let selected_workspace_id = self.selected_workspace.lock().unwrap().clone();
        Ok(WorkspaceState {
            selected_workspace_id,
            workspaces,
        })
    }

    fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        self.workspaces
            .lock()
            .unwrap()
            .insert(workspace.id.clone(), workspace);
        Ok(())
    }

    fn save_tab(&self, tab: Tab) -> Result<()> {
        self.tabs.lock().unwrap().insert(tab.id.clone(), tab);
        Ok(())
    }

    fn save_pane(&self, pane: Pane) -> Result<()> {
        self.panes.lock().unwrap().insert(pane.id.clone(), pane);
        Ok(())
    }

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        Ok(self.workspaces.lock().unwrap().get(id).cloned())
    }

    fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        Ok(self.tabs.lock().unwrap().get(id).cloned())
    }

    fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        Ok(self.panes.lock().unwrap().get(id).cloned())
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let mut workspaces: Vec<_> = self
            .workspaces
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        workspaces.sort_by_key(|w| w.position);
        Ok(workspaces)
    }

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        let mut tabs: Vec<_> = self
            .tabs
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.workspace_id == workspace_id)
            .cloned()
            .collect();
        tabs.sort_by_key(|t| t.position);
        Ok(tabs)
    }

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        let mut panes: Vec<_> = self
            .panes
            .lock()
            .unwrap()
            .values()
            .filter(|p| p.tab_id == tab_id)
            .cloned()
            .collect();
        panes.sort_by_key(|p| p.position);
        Ok(panes)
    }

    fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        self.workspaces.lock().unwrap().remove(workspace_id);
        Ok(())
    }

    fn delete_tab(&self, tab_id: &str) -> Result<()> {
        self.tabs.lock().unwrap().remove(tab_id);
        Ok(())
    }

    fn delete_pane(&self, pane_id: &str) -> Result<()> {
        self.panes.lock().unwrap().remove(pane_id);
        Ok(())
    }

    fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()> {
        *self.selected_workspace.lock().unwrap() = workspace_id;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkspaceKind;

    #[test]
    fn test_save_and_load_workspace() {
        let store = InMemoryWorkspaceStore::new();
        let ws = Workspace::new(WorkspaceKind::Folder, "/tmp/test".to_string());
        store.save_workspace(ws.clone()).unwrap();

        let loaded = store.load_workspace(&ws.id).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().path, "/tmp/test");
    }

    #[test]
    fn test_list_workspaces_sorted_by_position() {
        let store = InMemoryWorkspaceStore::new();

        let mut ws1 = Workspace::new(WorkspaceKind::Folder, "/tmp/test1".to_string());
        ws1.position = 1;
        store.save_workspace(ws1).unwrap();

        let mut ws2 = Workspace::new(WorkspaceKind::Folder, "/tmp/test2".to_string());
        ws2.position = 0;
        store.save_workspace(ws2.clone()).unwrap();

        let workspaces = store.list_workspaces().unwrap();
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].id, ws2.id);
    }

    #[test]
    fn test_selected_workspace() {
        let store = InMemoryWorkspaceStore::new();
        let ws = Workspace::new(WorkspaceKind::Folder, "/tmp/test".to_string());
        store.save_workspace(ws.clone()).unwrap();
        store.set_selected_workspace(Some(ws.id.clone())).unwrap();

        let state = store.load_state().unwrap();
        assert_eq!(state.selected_workspace_id, Some(ws.id));
    }
}
