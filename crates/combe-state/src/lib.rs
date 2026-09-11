use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceKind {
    #[serde(rename = "home")]
    Home,
    #[serde(rename = "repo")]
    Repo,
    #[serde(rename = "folder")]
    Folder,
    #[serde(rename = "worktree")]
    Worktree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub kind: WorkspaceKind,
    pub path: String,
    pub label: Option<String>,
}

impl Workspace {
    pub fn new(kind: WorkspaceKind, path: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            path,
            label: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: String,
    pub tab_id: String,
    pub cwd: String,
    pub parent_pane_id: Option<String>,
    pub split_direction: Option<SplitDirection>,
    pub split_ratio: Option<f64>,
    pub position: usize,
}

impl Pane {
    pub fn new(tab_id: String, cwd: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            tab_id,
            cwd,
            parent_pane_id: None,
            split_direction: None,
            split_ratio: None,
            position: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    #[serde(rename = "horizontal")]
    Horizontal,
    #[serde(rename = "vertical")]
    Vertical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub position: usize,
    pub focused_pane_id: Option<String>,
    pub panes: Vec<Pane>,
    pub zoom_pane_id: Option<String>,
}

impl Tab {
    pub fn new(workspace_id: String, title: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            workspace_id,
            title,
            position: 0,
            focused_pane_id: None,
            panes: Vec::new(),
            zoom_pane_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSession {
    pub workspace_id: String,
    pub tabs: Vec<Tab>,
    pub focused_tab_id: Option<String>,
}

impl WorkspaceSession {
    pub fn new(workspace_id: String) -> Self {
        Self {
            workspace_id,
            tabs: Vec::new(),
            focused_tab_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationState {
    pub version: String,
    pub selected_workspace_id: Option<String>,
    pub workspaces: Vec<Workspace>,
    pub sessions: Vec<WorkspaceSession>,
}

impl ApplicationState {
    pub fn new() -> Self {
        Self {
            version: "0.1.0".to_string(),
            selected_workspace_id: None,
            workspaces: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

impl Default for ApplicationState {
    fn default() -> Self {
        Self::new()
    }
}
