use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use combe_state::{FeltDbWorkspaceStore, Participant, WorkAssignment, WorkContext, WorkspaceStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("local participant provider '{0}' is not available")]
    Unavailable(String),
    #[error("worktree does not exist: {0}")]
    MissingWorktree(String),
    #[error("assignment is not addressed to participant {0}")]
    WrongParticipant(String),
    #[error("could not launch participant: {0}")]
    Launch(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalProvider {
    Codex,
    Claude,
}

impl LocalProvider {
    pub fn parse(value: &str) -> Result<Self, AdapterError> {
        match value.to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::Claude),
            other => Err(AdapterError::Unavailable(other.into())),
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantDescriptor {
    pub provider: LocalProvider,
    pub executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContext {
    pub worktree_path: String,
    pub repository_root: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPackage {
    pub schema: String,
    pub context: WorkContext,
    pub assignment: Option<WorkAssignment>,
    pub repository: RepositoryContext,
    pub constraints: Vec<String>,
}

impl ContextPackage {
    pub fn from_context(
        context: WorkContext,
        assignment: Option<WorkAssignment>,
    ) -> Result<Self, AdapterError> {
        let worktree_path = FeltDbWorkspaceStore::for_combe()
            .ok()
            .and_then(|store| {
                store
                    .load_workspace(&context.work.workspace_id)
                    .ok()
                    .flatten()
            })
            .map(|workspace| workspace.path)
            .unwrap_or_else(|| context.work.workspace_id.clone());
        let path = Path::new(&worktree_path);
        if !path.is_dir() {
            return Err(AdapterError::MissingWorktree(worktree_path));
        }
        let repository_root = git(path, &["rev-parse", "--show-toplevel"]);
        let branch = git(path, &["branch", "--show-current"]);
        Ok(Self {
            schema: "combe.work-context.v1".into(),
            repository: RepositoryContext {
                worktree_path,
                repository_root,
                branch,
            },
            context,
            assignment,
            constraints: vec![
                "Inspect the real worktree; repository files are authoritative.".into(),
                "Do not treat terminal or provider session state as durable Work context.".into(),
            ],
        })
    }

    pub fn text(&self) -> String {
        let context = &self.context;
        let mut output = format!(
            "WORK\nTitle: {}\nObjective: {}\nStatus: {:?}\nWorkspace ID: {}\nWork ID: {}\nWorktree path: {}\nRepository root: {}\nBranch: {}\n",
            context.work.title,
            context.work.objective.as_deref().unwrap_or(""),
            context.work.status,
            context.work.workspace_id,
            context.work.id,
            self.repository.worktree_path,
            self.repository.repository_root.as_deref().unwrap_or(""),
            self.repository.branch.as_deref().unwrap_or(""),
        );
        output.push_str("\nACTIVE ASSIGNMENT\n");
        if let Some(assignment) = &self.assignment {
            output.push_str(&format!(
                "From participant: {}\nTo participant: {}\nInstruction: {}\n",
                assignment.from_participant_id,
                assignment.to_participant_id,
                assignment.instruction
            ));
        } else {
            output.push_str("None\n");
        }
        output.push_str("\nCURRENT DECISIONS\n");
        for decision in &context.decisions {
            output.push_str(&format!("- {}\n", decision.statement));
        }
        output.push_str("\nRELEVANT ARTIFACTS\n");
        for artifact in &context.artifacts {
            if let Some(path) = &artifact.path {
                output.push_str(&format!("- {path}\n"));
            }
        }
        output.push_str("\nRECENT TURNS\n");
        for turn in &context.recent_turns {
            output.push_str(&format!("- {}: {}\n", turn.participant_id, turn.content));
        }
        output.push_str("\nCONSTRAINTS\n");
        for constraint in &self.constraints {
            output.push_str(&format!("- {constraint}\n"));
        }
        output.push_str("\nEXPECTED OUTCOME\n");
        if let Some(assignment) = &self.assignment {
            output.push_str(&assignment.instruction);
        }
        output.push('\n');
        output
    }
}

#[derive(Debug, Clone)]
pub struct PreparedHandoff {
    pub descriptor: ParticipantDescriptor,
    pub package: ContextPackage,
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub execution_id: String,
    pub exit_status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait ParticipantAdapter {
    fn prepare(
        &self,
        context: WorkContext,
        assignment: WorkAssignment,
        participant: &Participant,
    ) -> Result<PreparedHandoff, AdapterError>;
    fn launch(&self, handoff: PreparedHandoff) -> Result<ExecutionResult, AdapterError>;
}

pub struct LocalCliAdapter {
    descriptor: ParticipantDescriptor,
}

impl LocalCliAdapter {
    pub fn discover(provider: LocalProvider) -> Result<Self, AdapterError> {
        let executable = which::which(provider.executable())
            .map_err(|_| AdapterError::Unavailable(provider.executable().into()))?;
        Ok(Self {
            descriptor: ParticipantDescriptor {
                provider,
                executable,
            },
        })
    }
}

impl ParticipantAdapter for LocalCliAdapter {
    fn prepare(
        &self,
        context: WorkContext,
        assignment: WorkAssignment,
        participant: &Participant,
    ) -> Result<PreparedHandoff, AdapterError> {
        if assignment.to_participant_id != participant.id {
            return Err(AdapterError::WrongParticipant(participant.id.to_string()));
        }
        let package = ContextPackage::from_context(context, Some(assignment))?;
        Ok(PreparedHandoff {
            descriptor: self.descriptor.clone(),
            package,
        })
    }

    fn launch(&self, handoff: PreparedHandoff) -> Result<ExecutionResult, AdapterError> {
        let mut command = Command::new(&handoff.descriptor.executable);
        command
            .current_dir(&handoff.package.repository.worktree_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match handoff.descriptor.provider {
            LocalProvider::Codex => {
                command.args(["exec", "--ephemeral", "--sandbox", "workspace-write", "-"]);
            }
            LocalProvider::Claude => {
                command.args(["--print", "--no-session-persistence"]);
            }
        }
        let mut child = command.spawn()?;
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(handoff.package.text().as_bytes())?;
        let output = child.wait_with_output()?;
        Ok(ExecutionResult {
            execution_id: uuid::Uuid::new_v4().to_string(),
            exit_status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn git(path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use combe_state::{
        AssignmentId, AssignmentStatus, FeltDbWorkStore, ParticipantId, ParticipantKind,
        ParticipantResult, TurnId, TurnKind, Work, WorkId, WorkStatus, WorkStore, WorkTurn,
    };
    use tempfile::TempDir;

    fn fixture() -> (TempDir, WorkContext, WorkAssignment, Participant) {
        let directory = TempDir::new().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(directory.path())
            .status()
            .unwrap();
        let work = Work {
            id: WorkId("work-1".into()),
            workspace_id: directory.path().to_string_lossy().into_owned(),
            title: "Adapter test".into(),
            objective: Some("Prove handoff".into()),
            status: WorkStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let human = Participant {
            id: ParticipantId("human-1".into()),
            work_id: work.id.clone(),
            kind: ParticipantKind::Human,
            name: "Human".into(),
        };
        let agent = Participant {
            id: ParticipantId("agent-1".into()),
            work_id: work.id.clone(),
            kind: ParticipantKind::Agent,
            name: "Codex".into(),
        };
        let assignment = WorkAssignment {
            id: AssignmentId("assignment-1".into()),
            work_id: work.id.clone(),
            from_participant_id: human.id.clone(),
            to_participant_id: agent.id.clone(),
            instruction: "Do not modify files. Reply exactly COMBE_HANDOFF_OK.".into(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
        };
        let context = WorkContext {
            work,
            participants: vec![human, agent.clone()],
            recent_turns: Vec::new(),
            active_assignments: vec![assignment.clone()],
            decisions: Vec::new(),
            artifacts: Vec::new(),
        };
        (directory, context, assignment, agent)
    }

    #[test]
    fn context_package_is_deterministic_and_machine_readable() {
        let (_directory, context, assignment, _) = fixture();
        let package = ContextPackage::from_context(context, Some(assignment)).unwrap();
        assert_eq!(package.text(), package.text());
        let json = serde_json::to_string(&package).unwrap();
        assert_eq!(
            serde_json::from_str::<ContextPackage>(&json).unwrap(),
            package
        );
        assert!(json.contains("combe.work-context.v1"));
        assert!(package.text().contains("Worktree path:"));
    }

    #[test]
    fn installed_participant_is_discovered_and_missing_provider_is_clear() {
        assert!(LocalCliAdapter::discover(LocalProvider::Codex).is_ok());
        assert!(
            LocalProvider::parse("missing-provider")
                .unwrap_err()
                .to_string()
                .contains("not available")
        );
    }

    #[test]
    fn preparation_uses_work_workspace_as_worktree() {
        let (_directory, context, assignment, agent) = fixture();
        let adapter = LocalCliAdapter::discover(LocalProvider::Codex).unwrap();
        let expected = context.work.workspace_id.clone();
        let prepared = adapter.prepare(context, assignment, &agent).unwrap();
        assert_eq!(prepared.package.repository.worktree_path, expected);
    }

    #[test]
    #[ignore = "runs the installed Codex CLI"]
    fn real_codex_handoff_returns_output() {
        let (_directory, context, assignment, agent) = fixture();
        let database = TempDir::new().unwrap();
        let database_path = database.path().join("work.db");
        let store = FeltDbWorkStore::open(&database_path).unwrap();
        store
            .create_work(context.work.clone(), context.participants.clone())
            .unwrap();
        store.add_assignment(assignment.clone()).unwrap();
        let mut assignment = store
            .set_assignment_status(&assignment.id, AssignmentStatus::Active)
            .unwrap();
        let adapter = LocalCliAdapter::discover(LocalProvider::Codex).unwrap();
        let prepared = adapter
            .prepare(context, assignment.clone(), &agent)
            .unwrap();
        let result = adapter.launch(prepared).unwrap();
        assert_eq!(result.exit_status, Some(0));
        assert!(result.stdout.contains("COMBE_HANDOFF_OK"));
        assignment.status = AssignmentStatus::Completed;
        let now = Utc::now();
        let participant_result = ParticipantResult {
            id: "result-1".into(),
            work_id: assignment.work_id.clone(),
            participant_id: agent.id.clone(),
            assignment_id: assignment.id.clone(),
            execution_id: result.execution_id.clone(),
            exit_status: result.exit_status,
            summary: Some("COMBE_HANDOFF_OK".into()),
            output: Some(result.stdout.clone()),
            created_at: now,
        };
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: assignment.work_id.clone(),
            participant_id: agent.id,
            kind: TurnKind::Implementation,
            content: result.stdout,
            created_at: now,
            assignment_id: Some(assignment.id.clone()),
            execution_id: Some(result.execution_id),
        };
        store
            .finish_assignment(assignment.clone(), participant_result, turn, Vec::new())
            .unwrap();
        drop(store);
        let reopened = FeltDbWorkStore::open(database_path).unwrap();
        let restored = reopened.context(&assignment.work_id).unwrap();
        assert!(
            restored.recent_turns[0]
                .content
                .contains("COMBE_HANDOFF_OK")
        );
        assert!(restored.active_assignments.is_empty());
    }
}
