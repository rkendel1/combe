use crate::{Pane, Result, Tab, Workspace, WorkspaceState};

pub trait WorkspaceStore: Send + Sync {
    fn load_state(&self) -> Result<WorkspaceState>;

    fn save_workspace(&self, workspace: Workspace) -> Result<()>;

    fn save_tab(&self, tab: Tab) -> Result<()>;

    fn save_pane(&self, pane: Pane) -> Result<()>;

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>>;

    fn load_tab(&self, id: &str) -> Result<Option<Tab>>;

    fn load_pane(&self, id: &str) -> Result<Option<Pane>>;

    fn list_workspaces(&self) -> Result<Vec<Workspace>>;

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>>;

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>>;

    fn delete_workspace(&self, workspace_id: &str) -> Result<()>;

    fn delete_tab(&self, tab_id: &str) -> Result<()>;

    fn delete_pane(&self, pane_id: &str) -> Result<()>;

    fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()>;
}
