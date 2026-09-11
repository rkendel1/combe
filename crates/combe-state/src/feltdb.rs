use crate::{Pane, Result, StateError, Tab, Workspace, WorkspaceState};
use crate::store::WorkspaceStore;
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct FeltDbWorkspaceStore {
    data_dir: PathBuf,
}

impl FeltDbWorkspaceStore {
    pub fn open(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir).map_err(|source| StateError::Io {
            path: data_dir.clone(),
            source,
        })?;

        Ok(Self { data_dir })
    }

    pub fn default_path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("combe").join("feltdb"))
    }

    fn ensure_local_runtime(&self) -> Result<()> {
        todo!("Ensure local FeltDB runtime is started/available")
    }

    fn query_feltdb(&self, collection: &str, query: Value) -> Result<Value> {
        todo!("Query FeltDB through local protocol boundary")
    }

    fn mutate_feltdb(&self, collection: &str, operation: &str, data: Value) -> Result<Value> {
        todo!("Mutate FeltDB through local protocol boundary")
    }
}

impl WorkspaceStore for FeltDbWorkspaceStore {
    fn load_state(&self) -> Result<WorkspaceState> {
        self.ensure_local_runtime()?;

        let workspaces = self.list_workspaces()?;
        let selected_workspace_id = self
            .query_feltdb("session_metadata", json!({"key": "selected_workspace_id"}))?
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(WorkspaceState {
            selected_workspace_id,
            workspaces,
        })
    }

    fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb("workspaces", "upsert", serde_json::to_value(&workspace)?)?;
        Ok(())
    }

    fn save_tab(&self, tab: Tab) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb("tabs", "upsert", serde_json::to_value(&tab)?)?;
        Ok(())
    }

    fn save_pane(&self, pane: Pane) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb("panes", "upsert", serde_json::to_value(&pane)?)?;
        Ok(())
    }

    fn load_workspace(&self, id: &str) -> Result<Option<Workspace>> {
        self.ensure_local_runtime()?;
        let result = self.query_feltdb("workspaces", json!({"id": id}))?;
        Ok(serde_json::from_value(result).ok())
    }

    fn load_tab(&self, id: &str) -> Result<Option<Tab>> {
        self.ensure_local_runtime()?;
        let result = self.query_feltdb("tabs", json!({"id": id}))?;
        Ok(serde_json::from_value(result).ok())
    }

    fn load_pane(&self, id: &str) -> Result<Option<Pane>> {
        self.ensure_local_runtime()?;
        let result = self.query_feltdb("panes", json!({"id": id}))?;
        Ok(serde_json::from_value(result).ok())
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        self.ensure_local_runtime()?;
        let result = self.query_feltdb("workspaces", json!({}))?;
        let workspaces: Vec<Workspace> = serde_json::from_value(
            result.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )?;
        Ok(workspaces)
    }

    fn list_tabs(&self, workspace_id: &str) -> Result<Vec<Tab>> {
        self.ensure_local_runtime()?;
        let result =
            self.query_feltdb("tabs", json!({"workspace_id": workspace_id}))?;
        let tabs: Vec<Tab> = serde_json::from_value(
            result.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )?;
        Ok(tabs)
    }

    fn list_panes(&self, tab_id: &str) -> Result<Vec<Pane>> {
        self.ensure_local_runtime()?;
        let result = self.query_feltdb("panes", json!({"tab_id": tab_id}))?;
        let panes: Vec<Pane> = serde_json::from_value(
            result.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )?;
        Ok(panes)
    }

    fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb("workspaces", "delete", json!({"id": workspace_id}))?;
        Ok(())
    }

    fn delete_tab(&self, tab_id: &str) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb("tabs", "delete", json!({"id": tab_id}))?;
        Ok(())
    }

    fn delete_pane(&self, pane_id: &str) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb("panes", "delete", json!({"id": pane_id}))?;
        Ok(())
    }

    fn set_selected_workspace(&self, workspace_id: Option<String>) -> Result<()> {
        self.ensure_local_runtime()?;
        self.mutate_feltdb(
            "session_metadata",
            "upsert",
            json!({
                "key": "selected_workspace_id",
                "value": workspace_id
            }),
        )?;
        Ok(())
    }
}
