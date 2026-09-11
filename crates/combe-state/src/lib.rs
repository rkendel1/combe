pub mod ai_context;
pub mod attention;
pub mod chatgpt_history;
pub mod context_assembly;
pub mod context_graph;
pub mod error;
pub mod feltdb;
pub mod filestore;
pub mod memory;
pub mod migration;
pub mod protocol;
pub mod provider;
pub mod provider_store;
pub mod store;
pub mod work;
pub mod work_feltdb;
pub mod work_store;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use ai_context::*;
pub use attention::*;
pub use chatgpt_history::*;
pub use context_assembly::*;
pub use context_graph::*;
pub use error::{Result, StateError};
pub use feltdb::FeltDbWorkspaceStore;
pub use filestore::FileWorkspaceStore;
pub use memory::InMemoryWorkspaceStore;
pub use migration::migrate_from_json;
pub use protocol::*;
pub use provider::*;
pub use provider_store::ProviderRegistry;
pub use store::WorkspaceStore;
pub use work::*;
pub use work_feltdb::FeltDbWorkStore;
pub use work_store::WorkStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub kind: WorkspaceKind,
    pub path: String,
    pub label: Option<String>,
    pub position: u32,
}

impl Workspace {
    pub fn new(kind: WorkspaceKind, path: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            path,
            label: None,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub id: String,
    pub tab_id: String,
    pub cwd: String,
    pub split_parent_id: Option<String>,
    pub split_direction: Option<SplitDirection>,
    pub split_ratio: Option<f64>,
    pub position: u32,
}

impl Pane {
    pub fn new(tab_id: String, cwd: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            tab_id,
            cwd,
            split_parent_id: None,
            split_direction: None,
            split_ratio: None,
            position: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub position: u32,
    pub focused_pane_id: Option<String>,
}

impl Tab {
    pub fn new(workspace_id: String, title: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            workspace_id,
            title,
            position: 0,
            focused_pane_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub selected_workspace_id: Option<String>,
    pub workspaces: Vec<Workspace>,
}
