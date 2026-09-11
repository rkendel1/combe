use crate::error::Result;
use crate::*;
use std::path::PathBuf;

pub struct WorkspaceStore {
    path: PathBuf,
}

impl WorkspaceStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))
            .map_err(|source| crate::error::StateError::Io {
                path: path.to_path_buf(),
                source,
            })?;

        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn save_workspace(&mut self, _ws: &Workspace) -> Result<()> {
        todo!("Implement FeltDB-backed workspace persistence")
    }

    pub fn load_workspace(&self, _id: &str) -> Result<Option<Workspace>> {
        todo!("Implement FeltDB-backed workspace loading")
    }

    pub fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        todo!("Implement FeltDB-backed workspace listing")
    }

    pub fn save_tab(&mut self, _tab: &Tab) -> Result<()> {
        todo!("Implement FeltDB-backed tab persistence")
    }

    pub fn load_tab(&self, _id: &str) -> Result<Option<Tab>> {
        todo!("Implement FeltDB-backed tab loading")
    }

    pub fn list_tabs(&self, _workspace_id: &str) -> Result<Vec<Tab>> {
        todo!("Implement FeltDB-backed tab listing")
    }

    pub fn save_pane(&mut self, _pane: &Pane) -> Result<()> {
        todo!("Implement FeltDB-backed pane persistence")
    }

    pub fn load_pane(&self, _id: &str) -> Result<Option<Pane>> {
        todo!("Implement FeltDB-backed pane loading")
    }

    pub fn list_panes(&self, _tab_id: &str) -> Result<Vec<Pane>> {
        todo!("Implement FeltDB-backed pane listing")
    }

    pub fn state_path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("combe").join("feltdb"))
    }
}
