use combe_state::{FileWorkspaceStore, Tab, Workspace, WorkspaceState, WorkspaceStore, Result};
use std::path::Path;

pub struct SessionRecovery;

impl SessionRecovery {
    /// Load the last known workspace arrangement
    pub fn load_last_session() -> Result<WorkspaceState> {
        let store = FileWorkspaceStore::for_combe()?;
        store.load_state()
    }

    /// Validate that a workspace path still exists, fallback to workspace path if CWD missing
    pub fn validate_pane_cwd(pane_cwd: &str, workspace_path: &str) -> String {
        if Path::new(pane_cwd).is_dir() {
            pane_cwd.to_string()
        } else {
            workspace_path.to_string()
        }
    }

    /// Check if all workspaces in a session still exist
    pub fn validate_session(state: &WorkspaceState) -> SessionValidation {
        let mut validation = SessionValidation::default();

        for workspace in &state.workspaces {
            if Path::new(&workspace.path).is_dir() {
                validation.valid_workspaces.push(workspace.clone());
            } else {
                validation.missing_workspaces.push(workspace.path.clone());
            }
        }

        validation.is_valid = !validation.valid_workspaces.is_empty();
        validation
    }

    /// Generate a diagnostic report of session recovery
    pub fn recovery_report(validation: &SessionValidation) -> String {
        let mut report = String::new();

        if validation.valid_workspaces.is_empty() {
            report.push_str("Session recovery: No valid workspaces found\n");
        } else {
            report.push_str(&format!(
                "Session recovery: {} valid workspace(s)\n",
                validation.valid_workspaces.len()
            ));
        }

        if !validation.missing_workspaces.is_empty() {
            report.push_str(&format!(
                "Warning: {} workspace(s) no longer exist:\n",
                validation.missing_workspaces.len()
            ));
            for path in &validation.missing_workspaces {
                report.push_str(&format!("  - {}\n", path));
            }
        }

        report
    }
}

#[derive(Debug, Default, Clone)]
pub struct SessionValidation {
    pub is_valid: bool,
    pub valid_workspaces: Vec<Workspace>,
    pub missing_workspaces: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use combe_state::WorkspaceKind;

    #[test]
    fn test_validate_pane_cwd_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let result = SessionRecovery::validate_pane_cwd(path, "/fallback");
        assert_eq!(result, path);
    }

    #[test]
    fn test_validate_pane_cwd_missing_directory() {
        let result = SessionRecovery::validate_pane_cwd(
            "/nonexistent/path",
            "/fallback/workspace",
        );
        assert_eq!(result, "/fallback/workspace");
    }

    #[test]
    fn test_validate_session_with_missing_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let valid_path = dir.path().to_str().unwrap().to_string();

        let state = WorkspaceState {
            selected_workspace_id: None,
            workspaces: vec![
                Workspace {
                    id: "w1".to_string(),
                    kind: WorkspaceKind::Folder,
                    path: valid_path,
                    label: None,
                    position: 0,
                },
                Workspace {
                    id: "w2".to_string(),
                    kind: WorkspaceKind::Folder,
                    path: "/nonexistent/path".to_string(),
                    label: None,
                    position: 1,
                },
            ],
        };

        let validation = SessionRecovery::validate_session(&state);
        assert!(validation.is_valid);
        assert_eq!(validation.valid_workspaces.len(), 1);
        assert_eq!(validation.missing_workspaces.len(), 1);
    }

    #[test]
    fn test_recovery_report() {
        let validation = SessionValidation {
            is_valid: true,
            valid_workspaces: vec![],
            missing_workspaces: vec!["/tmp/old".to_string()],
        };
        let report = SessionRecovery::recovery_report(&validation);
        assert!(report.contains("no valid workspaces found"));
        assert!(report.contains("/tmp/old"));
    }
}
