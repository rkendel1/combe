use std::path::{Path, PathBuf};

use chrono::Utc;
use feltdb::{AtomicMutation, AtomicPrecondition};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use crate::work_store::{
    CONTEXT_ARTIFACT_LIMIT, CONTEXT_DECISION_LIMIT, CONTEXT_TEXT_LIMIT, CONTEXT_TURN_LIMIT,
};
use crate::{
    ArtifactId, AssignmentId, AssignmentStatus, Participant, ParticipantId, ParticipantResult,
    Result, StateError, TurnId, Work, WorkArtifact, WorkAssignment, WorkContext, WorkDecision,
    WorkId, WorkStatus, WorkStore, WorkTurn,
};

pub struct FeltDbWorkStore {
    db: feltdb::FeltDb,
}

impl FeltDbWorkStore {
    pub fn new() -> Result<Self> {
        let path = Self::database_path()?;
        Self::open(path)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = feltdb::FeltDb::open(path)
            .map_err(|error| StateError::FeltDbError(error.to_string()))?;
        Ok(Self { db })
    }

    pub fn for_combe() -> Result<Self> {
        Self::new()
    }

    fn database_path() -> Result<PathBuf> {
        let directory = dirs::data_dir()
            .ok_or_else(|| StateError::FeltDbError("cannot determine data directory".into()))?;
        Ok(directory.join("combe").join("workspace-data"))
    }

    fn key(collection: &str, id: &str) -> String {
        format!("combe/{collection}:{id}")
    }

    fn get<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<Option<T>> {
        self.db
            .get(&Self::key(collection, id))
            .map_err(|error| StateError::FeltDbError(error.to_string()))
    }

    fn insert<T: Serialize>(&self, collection: &str, id: &str, value: T) -> Result<()> {
        self.db
            .insert(&Self::key(collection, id), value)
            .map_err(|error| StateError::FeltDbError(error.to_string()))
    }

    fn scan<T: DeserializeOwned>(&self, collection: &str) -> Result<Vec<T>> {
        self.db
            .query_collection(&format!("combe/{collection}"), None, |_| true)
            .map_err(|error| StateError::FeltDbError(error.to_string()))?
            .into_iter()
            .map(|row| serde_json::from_value(row.value).map_err(StateError::from))
            .collect()
    }

    fn mutation<T: Serialize>(collection: &str, id: &str, value: T) -> Result<AtomicMutation> {
        Ok(AtomicMutation {
            capability: format!("combe/{collection}"),
            key: Self::key(collection, id),
            value: Some(serde_json::to_value(value)?),
        })
    }

    fn absent(collection: &str, id: &str) -> AtomicPrecondition {
        AtomicPrecondition {
            capability: format!("combe/{collection}"),
            key: Self::key(collection, id),
            expected_version: None,
        }
    }

    fn atomic(
        &self,
        mutations: Vec<AtomicMutation>,
        absent: Vec<AtomicPrecondition>,
    ) -> Result<()> {
        self.db
            .apply_atomic_transaction(
                &format!("combe-{}", Uuid::new_v4()),
                None,
                &absent,
                &mutations,
                Some(json!({ "application": "combe" })),
            )
            .map_err(|error| StateError::FeltDbError(error.to_string()))?;
        Ok(())
    }

    fn require_work(&self, id: &WorkId) -> Result<Work> {
        self.load_work(id)?.ok_or_else(|| StateError::NotFound {
            entity_type: "work".into(),
            id: id.0.clone(),
        })
    }

    fn validate_participant(&self, work_id: &WorkId, participant_id: &ParticipantId) -> Result<()> {
        let participant = self.get::<Participant>("participant", &participant_id.0)?;
        if participant
            .as_ref()
            .is_some_and(|participant| participant.work_id == *work_id)
        {
            Ok(())
        } else {
            Err(StateError::InvalidEntity(format!(
                "participant {participant_id} does not belong to work {work_id}"
            )))
        }
    }
}

impl WorkStore for FeltDbWorkStore {
    fn create_work(&self, work: Work, participants: Vec<Participant>) -> Result<()> {
        if work.title.trim().is_empty() {
            return Err(StateError::InvalidEntity(
                "work title cannot be empty".into(),
            ));
        }
        if participants
            .iter()
            .any(|participant| participant.work_id != work.id)
        {
            return Err(StateError::InvalidEntity(
                "participant belongs to another work".into(),
            ));
        }
        let mut mutations = vec![Self::mutation("work", &work.id.0, &work)?];
        let mut preconditions = vec![Self::absent("work", &work.id.0)];
        for participant in participants {
            mutations.push(Self::mutation(
                "participant",
                &participant.id.0,
                &participant,
            )?);
            preconditions.push(Self::absent("participant", &participant.id.0));
        }
        self.atomic(mutations, preconditions)
    }

    fn load_work(&self, id: &WorkId) -> Result<Option<Work>> {
        self.get("work", &id.0)
    }

    fn list_works(&self, workspace_id: Option<&str>) -> Result<Vec<Work>> {
        let workspace_id = workspace_id.map(str::to_owned);
        let mut works: Vec<Work> = self.scan("work")?;
        works.retain(|work| {
            workspace_id
                .as_ref()
                .is_none_or(|id| work.workspace_id == *id)
        });
        works.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(works)
    }

    fn set_work_status(&self, id: &WorkId, status: WorkStatus) -> Result<Work> {
        let mut work = self.require_work(id)?;
        work.status = status;
        work.updated_at = Utc::now();
        self.insert("work", &id.0, &work)?;
        Ok(work)
    }

    fn add_participant(&self, participant: Participant) -> Result<()> {
        self.require_work(&participant.work_id)?;
        self.insert("participant", &participant.id.0, &participant)
    }

    fn participants(&self, work_id: &WorkId) -> Result<Vec<Participant>> {
        let mut values: Vec<Participant> = self.scan("participant")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn add_turn(&self, turn: WorkTurn) -> Result<()> {
        self.require_work(&turn.work_id)?;
        self.validate_participant(&turn.work_id, &turn.participant_id)?;
        self.insert("turn", &turn.id.0, &turn)
    }

    fn turns(&self, work_id: &WorkId) -> Result<Vec<WorkTurn>> {
        let mut values: Vec<WorkTurn> = self.scan("turn")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn add_assignment(&self, assignment: WorkAssignment) -> Result<()> {
        self.require_work(&assignment.work_id)?;
        self.validate_participant(&assignment.work_id, &assignment.from_participant_id)?;
        self.validate_participant(&assignment.work_id, &assignment.to_participant_id)?;
        self.insert("assignment", &assignment.id.0, &assignment)
    }

    fn assignments(&self, work_id: &WorkId) -> Result<Vec<WorkAssignment>> {
        let mut values: Vec<WorkAssignment> = self.scan("assignment")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn set_assignment_status(
        &self,
        id: &AssignmentId,
        status: AssignmentStatus,
    ) -> Result<WorkAssignment> {
        let mut assignment = self
            .get::<WorkAssignment>("assignment", &id.0)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "assignment".into(),
                id: id.0.clone(),
            })?;
        assignment.status = status;
        self.insert("assignment", &id.0, &assignment)?;
        Ok(assignment)
    }

    fn record_decision(&self, decision: WorkDecision, turn: Option<WorkTurn>) -> Result<()> {
        self.require_work(&decision.work_id)?;
        self.validate_participant(&decision.work_id, &decision.decided_by)?;
        if turn.as_ref().is_some_and(|turn| {
            turn.work_id != decision.work_id || turn.participant_id != decision.decided_by
        }) {
            return Err(StateError::InvalidEntity(
                "decision turn provenance does not match".into(),
            ));
        }
        let mut mutations = vec![Self::mutation("decision", &decision.id.0, &decision)?];
        let mut preconditions = vec![Self::absent("decision", &decision.id.0)];
        if let Some(turn) = turn {
            mutations.push(Self::mutation("turn", &turn.id.0, &turn)?);
            preconditions.push(Self::absent("turn", &turn.id.0));
        }
        self.atomic(mutations, preconditions)
    }

    fn decisions(&self, work_id: &WorkId) -> Result<Vec<WorkDecision>> {
        let mut values: Vec<WorkDecision> = self.scan("decision")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn add_artifact(&self, artifact: WorkArtifact) -> Result<()> {
        self.require_work(&artifact.work_id)?;
        self.validate_participant(&artifact.work_id, &artifact.created_by)?;
        self.insert("artifact", &artifact.id.0, &artifact)
    }

    fn artifacts(&self, work_id: &WorkId) -> Result<Vec<WorkArtifact>> {
        let mut values: Vec<WorkArtifact> = self.scan("artifact")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn complete_assignment(
        &self,
        assignment: WorkAssignment,
        turn: WorkTurn,
        artifacts: Vec<WorkArtifact>,
    ) -> Result<()> {
        if assignment.status != AssignmentStatus::Completed
            || turn.work_id != assignment.work_id
            || turn.participant_id != assignment.to_participant_id
        {
            return Err(StateError::InvalidEntity(
                "assignment completion provenance does not match".into(),
            ));
        }
        self.validate_participant(&assignment.work_id, &assignment.to_participant_id)?;
        if artifacts.iter().any(|artifact| {
            artifact.work_id != assignment.work_id
                || artifact.created_by != assignment.to_participant_id
        }) {
            return Err(StateError::InvalidEntity(
                "assignment artifact provenance does not match".into(),
            ));
        }
        let mut mutations = vec![
            Self::mutation("assignment", &assignment.id.0, &assignment)?,
            Self::mutation("turn", &turn.id.0, &turn)?,
        ];
        let mut preconditions = vec![Self::absent("turn", &turn.id.0)];
        for artifact in artifacts {
            mutations.push(Self::mutation("artifact", &artifact.id.0, &artifact)?);
            preconditions.push(Self::absent("artifact", &artifact.id.0));
        }
        self.atomic(mutations, preconditions)
    }

    fn finish_assignment(
        &self,
        assignment: WorkAssignment,
        result: ParticipantResult,
        turn: WorkTurn,
        artifacts: Vec<WorkArtifact>,
    ) -> Result<()> {
        if !matches!(
            assignment.status,
            AssignmentStatus::Completed | AssignmentStatus::Failed
        ) || result.work_id != assignment.work_id
            || result.assignment_id != assignment.id
            || result.participant_id != assignment.to_participant_id
            || turn.work_id != assignment.work_id
            || turn.participant_id != assignment.to_participant_id
            || turn.assignment_id.as_ref() != Some(&assignment.id)
            || turn.execution_id.as_deref() != Some(result.execution_id.as_str())
        {
            return Err(StateError::InvalidEntity(
                "participant result provenance does not match assignment".into(),
            ));
        }
        self.validate_participant(&assignment.work_id, &assignment.to_participant_id)?;
        if artifacts.iter().any(|artifact| {
            artifact.work_id != assignment.work_id
                || artifact.created_by != assignment.to_participant_id
        }) {
            return Err(StateError::InvalidEntity(
                "result artifact provenance does not match".into(),
            ));
        }
        let mut mutations = vec![
            Self::mutation("assignment", &assignment.id.0, &assignment)?,
            Self::mutation("result", &result.id, &result)?,
            Self::mutation("turn", &turn.id.0, &turn)?,
        ];
        let mut preconditions = vec![
            Self::absent("result", &result.id),
            Self::absent("turn", &turn.id.0),
        ];
        for artifact in artifacts {
            mutations.push(Self::mutation("artifact", &artifact.id.0, &artifact)?);
            preconditions.push(Self::absent("artifact", &artifact.id.0));
        }
        self.atomic(mutations, preconditions)
    }

    fn context(&self, work_id: &WorkId) -> Result<WorkContext> {
        let work = self.require_work(work_id)?;
        let participants = self.participants(work_id)?;
        let turns = self.turns(work_id)?;
        let recent_turns = turns
            .into_iter()
            .rev()
            .take(CONTEXT_TURN_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|mut turn| {
                turn.content = bounded(&turn.content);
                turn
            })
            .collect();
        let active_assignments = self
            .assignments(work_id)?
            .into_iter()
            .filter(|assignment| {
                matches!(
                    assignment.status,
                    AssignmentStatus::Pending | AssignmentStatus::Active
                )
            })
            .collect();
        let decisions = self
            .decisions(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_DECISION_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let artifacts = self
            .artifacts(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_ARTIFACT_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Ok(WorkContext {
            work,
            participants,
            recent_turns,
            active_assignments,
            decisions,
            artifacts,
        })
    }

    fn load_turn(&self, id: &TurnId) -> Result<Option<WorkTurn>> {
        self.get("turn", &id.0)
    }
    fn load_participant(&self, id: &ParticipantId) -> Result<Option<Participant>> {
        self.get("participant", &id.0)
    }
    fn load_artifact(&self, id: &ArtifactId) -> Result<Option<WorkArtifact>> {
        self.get("artifact", &id.0)
    }
    fn load_result(&self, id: &str) -> Result<Option<ParticipantResult>> {
        self.get("result", id)
    }
}

fn bounded(value: &str) -> String {
    if value.len() <= CONTEXT_TEXT_LIMIT {
        return value.to_string();
    }
    let mut end = CONTEXT_TEXT_LIMIT;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactKind, DecisionId, ParticipantKind, TurnKind};
    use tempfile::TempDir;

    fn store() -> (FeltDbWorkStore, TempDir) {
        let directory = TempDir::new().unwrap();
        let store = FeltDbWorkStore::open(directory.path().join("work.db")).unwrap();
        (store, directory)
    }

    fn work_with_participants(store: &FeltDbWorkStore) -> (Work, Participant, Participant) {
        let work = Work::new(
            "workspace-1".into(),
            "Foundation".into(),
            Some("Coordinate work".into()),
        );
        let human = Participant::new(work.id.clone(), ParticipantKind::Human, "Randy".into());
        let agent = Participant::new(work.id.clone(), ParticipantKind::Agent, "Reviewer".into());
        store
            .create_work(work.clone(), vec![human.clone(), agent.clone()])
            .unwrap();
        (work, human, agent)
    }

    #[test]
    fn lifecycle_and_participants_persist_after_reopen() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let work = {
            let store = FeltDbWorkStore::open(&path).unwrap();
            let (work, _, _) = work_with_participants(&store);
            store
                .set_work_status(&work.id, WorkStatus::Completed)
                .unwrap();
            work
        };
        let store = FeltDbWorkStore::open(&path).unwrap();
        assert_eq!(
            store.load_work(&work.id).unwrap().unwrap().status,
            WorkStatus::Completed
        );
        assert_eq!(store.participants(&work.id).unwrap().len(), 2);
        assert_eq!(
            store
                .set_work_status(&work.id, WorkStatus::Archived)
                .unwrap()
                .status,
            WorkStatus::Archived
        );
    }

    #[test]
    fn turns_assignments_decisions_and_artifacts_reconstruct_context() {
        let (store, _directory) = store();
        let (work, human, agent) = work_with_participants(&store);
        store
            .add_participant(Participant::new(
                work.id.clone(),
                ParticipantKind::System,
                "Combe".into(),
            ))
            .unwrap();
        for content in ["first", "second"] {
            store
                .add_turn(WorkTurn {
                    id: TurnId::new(),
                    work_id: work.id.clone(),
                    participant_id: agent.id.clone(),
                    kind: TurnKind::Review,
                    content: content.into(),
                    created_at: Utc::now(),
                    assignment_id: None,
                    execution_id: None,
                })
                .unwrap();
        }
        let assignment = WorkAssignment {
            id: AssignmentId::new(),
            work_id: work.id.clone(),
            from_participant_id: human.id.clone(),
            to_participant_id: agent.id.clone(),
            instruction: "Review".into(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
        };
        store.add_assignment(assignment.clone()).unwrap();
        store
            .set_assignment_status(&assignment.id, AssignmentStatus::Active)
            .unwrap();
        let decision = WorkDecision {
            id: DecisionId::new(),
            work_id: work.id.clone(),
            statement: "Use FeltDB".into(),
            rationale: Some("One authority".into()),
            decided_by: human.id.clone(),
            created_at: Utc::now(),
        };
        store.record_decision(decision, None).unwrap();
        store
            .add_artifact(WorkArtifact {
                id: ArtifactId::new(),
                work_id: work.id.clone(),
                kind: ArtifactKind::File,
                path: Some("src/work.rs".into()),
                description: None,
                created_by: agent.id,
                created_at: Utc::now(),
            })
            .unwrap();
        let context = store.context(&work.id).unwrap();
        assert_eq!(context.participants.len(), 3);
        assert_eq!(
            context
                .recent_turns
                .iter()
                .map(|turn| turn.content.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(context.active_assignments.len(), 1);
        assert_eq!(context.decisions.len(), 1);
        assert_eq!(context.artifacts.len(), 1);
        assert_eq!(
            store
                .set_assignment_status(&assignment.id, AssignmentStatus::Cancelled)
                .unwrap()
                .status,
            AssignmentStatus::Cancelled
        );
    }

    #[test]
    fn assignment_completion_is_atomic_and_rejects_partial_invalid_input() {
        let (store, _directory) = store();
        let (work, human, agent) = work_with_participants(&store);
        let assignment = WorkAssignment {
            id: AssignmentId::new(),
            work_id: work.id.clone(),
            from_participant_id: human.id,
            to_participant_id: agent.id.clone(),
            instruction: "Implement".into(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
        };
        store.add_assignment(assignment.clone()).unwrap();
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: work.id.clone(),
            participant_id: agent.id.clone(),
            kind: TurnKind::Implementation,
            content: "Done".into(),
            created_at: Utc::now(),
            assignment_id: None,
            execution_id: None,
        };
        let artifact = WorkArtifact {
            id: ArtifactId::new(),
            work_id: work.id.clone(),
            kind: ArtifactKind::Patch,
            path: Some("change.patch".into()),
            description: None,
            created_by: agent.id,
            created_at: Utc::now(),
        };
        assert!(
            store
                .complete_assignment(assignment.clone(), turn.clone(), vec![artifact.clone()])
                .is_err()
        );
        assert!(store.load_turn(&turn.id).unwrap().is_none());
        assert!(store.load_artifact(&artifact.id).unwrap().is_none());
        let mut completed = assignment;
        completed.status = AssignmentStatus::Completed;
        store
            .complete_assignment(completed, turn.clone(), vec![artifact.clone()])
            .unwrap();
        assert!(store.load_turn(&turn.id).unwrap().is_some());
        assert!(store.load_artifact(&artifact.id).unwrap().is_some());
    }

    #[test]
    fn participant_result_and_provenance_turn_persist_after_reopen() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let (work_id, turn_id) = {
            let store = FeltDbWorkStore::open(&path).unwrap();
            let (work, human, agent) = work_with_participants(&store);
            let assignment = WorkAssignment {
                id: AssignmentId::new(),
                work_id: work.id.clone(),
                from_participant_id: human.id,
                to_participant_id: agent.id.clone(),
                instruction: "Review".into(),
                status: AssignmentStatus::Failed,
                created_at: Utc::now(),
            };
            store.add_assignment(assignment.clone()).unwrap();
            let execution_id = "execution-1".to_string();
            let result = ParticipantResult {
                id: "result-1".into(),
                work_id: work.id.clone(),
                participant_id: agent.id.clone(),
                assignment_id: assignment.id.clone(),
                execution_id: execution_id.clone(),
                exit_status: Some(7),
                summary: Some("review failed".into()),
                output: Some("details".into()),
                created_at: Utc::now(),
            };
            let turn = WorkTurn {
                id: TurnId::new(),
                work_id: work.id.clone(),
                participant_id: agent.id,
                kind: TurnKind::Review,
                content: "details".into(),
                created_at: Utc::now(),
                assignment_id: Some(assignment.id.clone()),
                execution_id: Some(execution_id),
            };
            store
                .finish_assignment(assignment, result, turn.clone(), Vec::new())
                .unwrap();
            (work.id, turn.id)
        };
        let store = FeltDbWorkStore::open(&path).unwrap();
        let turn = store.load_turn(&turn_id).unwrap().unwrap();
        assert_eq!(turn.execution_id.as_deref(), Some("execution-1"));
        assert_eq!(
            store.load_result("result-1").unwrap().unwrap().exit_status,
            Some(7)
        );
        assert_eq!(
            store.assignments(&work_id).unwrap()[0].status,
            AssignmentStatus::Failed
        );
    }
}
