use crate::client::FeltDbLocalClient;
use crate::protocol::TransactOperation;
use crate::{Pane, Result, Tab, Workspace, WorkspaceState};
use crate::store::WorkspaceStore;
use serde_json::{json, Value};

pub struct FeltDbWorkspaceStore {
    client: FeltDbLocalClient,
}

impl FeltDbWorkspaceStore {
    pub fn connect() -> Result<Self> {
        let client = FeltDbLocalClient::for_combe()?;
        Ok(Self { client })
    }
}

impl WorkspaceStore for FeltDbWorkspaceStore {
    fn load_state(&self) -> Result<WorkspaceState> {
        let workspaces = self.list_workspaces()?;
        let selected_workspace_id = self
            .client
            .query("session_metadata", json!({"key": "selected_workspace_id"}))?
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(WorkspaceState {
            selected_workspace_id,
            workspaces,
        })
    }

    fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        self.client
            .upsert("workspaces", serde_json::to_value(&workspace)?)?;
        Ok(())
    }

    fn save_tab(&self, tab: Tab) -> Result<()> {
        self.client
            .upsert("tabs", serde_json::to_value(&tab)?)?;
        Ok(())
    }

    fn save_pane(&self, pane: Pane) -> Result<()> {
        self.client
            .upsert("panes", serde_json::to_value(&pane)?)?;
        Ok(())
    }

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        let result = self.client.query("workspaces", json!({"id": id}))?;
        Ok(serde_json::from_value(result).ok())
    }

    fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        let result = self.client.query("tabs", json!({"id": id}))?;
        Ok(serde_json::from_value(result).ok())
    }

    fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        let result = self.client.query("panes", json!({"id": id}))?;
        Ok(serde_json::from_value(result).ok())
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let result = self.client.query("workspaces", json!({}))?;
        let workspaces: Vec<Workspace> = serde_json::from_value(
            result.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )?;
        Ok(workspaces)
    }

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        let result = self.client.query("tabs", json!({"workspace_id": workspace_id}))?;
        let tabs: Vec<Tab> = serde_json::from_value(
            result.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )?;
        Ok(tabs)
    }

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        let result = self.client.query("panes", json!({"tab_id": tab_id}))?;
        let panes: Vec<Pane> = serde_json::from_value(
            result.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )?;
        Ok(panes)
    }

    fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        self.client.delete("workspaces", workspace_id)
    }

    fn delete_tab(&self, tab_id: &str) -> Result<()> {
        self.client.delete("tabs", tab_id)
    }

    fn delete_pane(&self, pane_id: &str) -> Result<()> {
        self.client.delete("panes", pane_id)
    }

    fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()> {
        let operation = TransactOperation::Upsert {
            collection: "session_metadata".to_string(),
            document: json!({
                "key": "selected_workspace_id",
                "value": workspace_id
            }),
        };
        self.client.transact(vec![operation])?;
        Ok(())
    }
}
