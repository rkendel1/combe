use std::path::{Path, PathBuf};

use chrono::Utc;
use feltdb::{AtomicMutation, AtomicPrecondition};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use crate::work_store::{
    CONTEXT_ARTIFACT_LIMIT, CONTEXT_ASSIGNMENT_LIMIT, CONTEXT_CONVERSATION_LIMIT,
    CONTEXT_DECISION_LIMIT, CONTEXT_EXECUTION_LIMIT, CONTEXT_EXECUTION_REVIEW_LIMIT,
    CONTEXT_PROPOSAL_LIMIT, CONTEXT_RESULT_LIMIT, CONTEXT_REVIEW_LIMIT, CONTEXT_TEXT_LIMIT,
    CONTEXT_TURN_LIMIT, EXECUTION_STALE_AFTER_MINUTES,
};
use crate::{
    ArtifactId, AssignmentId, AssignmentStatus, ContributionAcceptance, ContributionKind,
    ConversationId, ExecutionId, ExecutionReview, ExecutionReviewId, ExecutionStatus, Participant,
    ParticipantId, ParticipantResult, ProposalId, ProposalReview, ProposalStatus, Result, ReviewId,
    ReviewOutcome, StateError, TurnId, TurnKind, TurnOrigin, WORK_CONTEXT_VERSION, Work,
    WorkArtifact, WorkAssignment, WorkContext, WorkContribution, WorkConversation, WorkDecision,
    WorkExecution, WorkId, WorkProposal, WorkState, WorkStatus, WorkStore, WorkTurn,
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
        if collection == "work-conversation" {
            format!("combe/work-conversation/{id}")
        } else {
            format!("combe/{collection}:{id}")
        }
    }

    fn get<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<Option<T>> {
        if collection == "work-conversation" {
            return self
                .db
                .get_collection_record(&collection_key(collection), &Self::key(collection, id))
                .map_err(|error| StateError::FeltDbError(error.to_string()))?
                .map(|row| serde_json::from_value(row.value).map_err(StateError::from))
                .transpose();
        }
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
            .query_collection(&collection_key(collection), None, |_| true)
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

    fn version(&self, collection: &str, id: &str) -> Result<u64> {
        self.db
            .get_collection_record(&collection_key(collection), &Self::key(collection, id))
            .map_err(|error| StateError::FeltDbError(error.to_string()))?
            .and_then(|row| row.operation.map(|operation| operation.sequence))
            .ok_or_else(|| StateError::NotFound {
                entity_type: collection.into(),
                id: id.into(),
            })
    }

    fn at_version(collection: &str, id: &str, version: u64) -> AtomicPrecondition {
        AtomicPrecondition {
            capability: format!("combe/{collection}"),
            key: Self::key(collection, id),
            expected_version: Some(version),
        }
    }

    fn atomic(
        &self,
        mutations: Vec<AtomicMutation>,
        preconditions: Vec<AtomicPrecondition>,
    ) -> Result<()> {
        self.atomic_at(None, mutations, preconditions)
    }

    fn atomic_at(
        &self,
        expected_revision: Option<u64>,
        mutations: Vec<AtomicMutation>,
        preconditions: Vec<AtomicPrecondition>,
    ) -> Result<()> {
        if let Some(expected) = expected_revision {
            let current = self.current_revision()?;
            if current != expected {
                return Err(StateError::StaleContext { expected, current });
            }
        }
        let outcome = self.db.apply_atomic_transaction(
            &format!("combe-{}", Uuid::new_v4()),
            expected_revision,
            &preconditions,
            &mutations,
            Some(json!({ "application": "combe" })),
        );
        match outcome {
            Ok(_) => Ok(()),
            Err(error) => {
                if let Some(expected) = expected_revision {
                    let current = self.current_revision()?;
                    if current != expected {
                        return Err(StateError::StaleContext { expected, current });
                    }
                }
                Err(StateError::FeltDbError(error.to_string()))
            }
        }
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

    fn validate_origin(
        &self,
        work_id: &WorkId,
        participant_id: &ParticipantId,
        origin: &TurnOrigin,
    ) -> Result<()> {
        if let TurnOrigin::ExternalConversation(reference) = origin
            && !self.conversations(work_id)?.into_iter().any(|link| {
                link.participant_id == *participant_id && link.conversation == *reference
            })
        {
            return Err(StateError::InvalidEntity(
                "external provenance is not linked to this participant and work".into(),
            ));
        }
        Ok(())
    }
}

fn collection_key(collection: &str) -> String {
    format!("combe/{collection}")
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
        if turn.origin != TurnOrigin::Local {
            return Err(StateError::InvalidEntity(
                "external turns must be imported with a conversation link".into(),
            ));
        }
        validate_contribution(&turn.content)?;
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
        if assignment.status != AssignmentStatus::Pending {
            return Err(StateError::InvalidEntity(
                "new assignment must be pending".into(),
            ));
        }
        self.validate_participant(&assignment.work_id, &assignment.from_participant_id)?;
        self.validate_participant(&assignment.work_id, &assignment.to_participant_id)?;
        if let Some(proposal_id) = &assignment.proposal_id {
            let proposal =
                self.load_proposal(proposal_id)?
                    .ok_or_else(|| StateError::NotFound {
                        entity_type: "proposal".into(),
                        id: proposal_id.0.clone(),
                    })?;
            if proposal.work_id != assignment.work_id || proposal.status != ProposalStatus::Approved
            {
                return Err(StateError::InvalidEntity(
                    "assignment proposal must be approved and belong to the work".into(),
                ));
            }
        }
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

    fn load_assignment(&self, id: &AssignmentId) -> Result<Option<WorkAssignment>> {
        self.get("assignment", &id.0)
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
            .collect::<Vec<_>>();
        let assignments = self
            .assignments(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_ASSIGNMENT_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let active_assignments = assignments
            .iter()
            .cloned()
            .into_iter()
            .filter(|assignment| {
                matches!(
                    assignment.status,
                    AssignmentStatus::Pending | AssignmentStatus::Active
                )
            })
            .collect::<Vec<_>>();
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
        let conversations = self
            .conversations(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_CONVERSATION_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let proposals = self
            .proposals(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_PROPOSAL_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let mut reviews = Vec::new();
        for proposal in &proposals {
            reviews.extend(self.reviews(&proposal.id)?);
        }
        reviews.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        reviews = reviews
            .into_iter()
            .rev()
            .take(CONTEXT_REVIEW_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let state = WorkState {
            active_proposals: proposals
                .iter()
                .filter(|proposal| {
                    matches!(
                        proposal.status,
                        ProposalStatus::Draft | ProposalStatus::Proposed
                    )
                })
                .cloned()
                .collect(),
            approved_proposals: proposals
                .iter()
                .filter(|proposal| proposal.status == ProposalStatus::Approved)
                .cloned()
                .collect(),
            active_assignments: active_assignments.clone(),
            recent_results: recent_turns
                .iter()
                .filter(|turn| turn.assignment_id.is_some() && turn.execution_id.is_some())
                .cloned()
                .collect(),
            assigned: active_assignments
                .iter()
                .filter(|assignment| assignment.status == AssignmentStatus::Pending)
                .cloned()
                .collect(),
            active_executions: Vec::new(),
            stale_executions: Vec::new(),
            completed_executions: Vec::new(),
            failed_executions: Vec::new(),
            cancelled_executions: Vec::new(),
        };
        let executions = self
            .executions(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_EXECUTION_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let stale_before = Utc::now() - chrono::Duration::minutes(EXECUTION_STALE_AFTER_MINUTES);
        let mut state = state;
        for execution in &executions {
            match execution.status {
                ExecutionStatus::Started => {
                    if execution.heartbeat_at.unwrap_or(execution.started_at) < stale_before {
                        state.stale_executions.push(execution.clone());
                    } else {
                        state.active_executions.push(execution.clone());
                    }
                }
                ExecutionStatus::Completed => state.completed_executions.push(execution.clone()),
                ExecutionStatus::Failed => state.failed_executions.push(execution.clone()),
                ExecutionStatus::Cancelled => state.cancelled_executions.push(execution.clone()),
            }
        }
        let mut results = executions
            .iter()
            .rev()
            .filter_map(|execution| execution.result_id.as_deref())
            .take(CONTEXT_RESULT_LIMIT)
            .filter_map(|id| self.load_result(id).transpose())
            .collect::<Result<Vec<_>>>()?;
        results.reverse();
        let execution_reviews = self
            .execution_reviews(work_id)?
            .into_iter()
            .rev()
            .take(CONTEXT_EXECUTION_REVIEW_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Ok(WorkContext {
            context_version: WORK_CONTEXT_VERSION,
            revision: self.current_revision()?,
            work,
            participants,
            recent_turns,
            active_assignments,
            decisions,
            artifacts,
            conversations,
            proposals,
            reviews,
            state,
            executions,
            assignments,
            results,
            execution_reviews,
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

    fn conversations(&self, work_id: &WorkId) -> Result<Vec<WorkConversation>> {
        let mut values: Vec<WorkConversation> = self.scan("work-conversation")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn import_contribution(&self, conversation: WorkConversation, turn: WorkTurn) -> Result<()> {
        self.require_work(&conversation.work_id)?;
        self.validate_participant(&conversation.work_id, &conversation.participant_id)?;
        validate_contribution(&turn.content)?;
        if turn.work_id != conversation.work_id
            || turn.participant_id != conversation.participant_id
            || turn.origin != TurnOrigin::ExternalConversation(conversation.conversation.clone())
        {
            return Err(StateError::InvalidEntity(
                "imported turn provenance does not match conversation".into(),
            ));
        }
        let existing = self.load_conversation(&conversation.id)?;
        if existing
            .as_ref()
            .is_some_and(|value| value != &conversation)
        {
            return Err(StateError::InvalidEntity(
                "conversation id already belongs to another link".into(),
            ));
        }
        let mut mutations = vec![Self::mutation("turn", &turn.id.0, &turn)?];
        let mut preconditions = vec![Self::absent("turn", &turn.id.0)];
        if existing.is_none() {
            mutations.push(Self::mutation(
                "work-conversation",
                &conversation.id.0,
                &conversation,
            )?);
            preconditions.push(Self::absent("work-conversation", &conversation.id.0));
        }
        self.atomic(mutations, preconditions)
    }

    fn load_conversation(&self, id: &ConversationId) -> Result<Option<WorkConversation>> {
        self.get("work-conversation", &id.0)
    }

    fn create_proposal(&self, proposal: WorkProposal) -> Result<()> {
        self.require_work(&proposal.work_id)?;
        self.validate_participant(&proposal.work_id, &proposal.proposed_by)?;
        self.validate_origin(&proposal.work_id, &proposal.proposed_by, &proposal.origin)?;
        if proposal.title.trim().is_empty() || proposal.statement.trim().is_empty() {
            return Err(StateError::InvalidEntity(
                "proposal title and statement cannot be empty".into(),
            ));
        }
        validate_contribution(&proposal.statement)?;
        if !matches!(
            proposal.status,
            ProposalStatus::Draft | ProposalStatus::Proposed
        ) {
            return Err(StateError::InvalidEntity(
                "new proposal must be draft or proposed".into(),
            ));
        }
        self.atomic(
            vec![Self::mutation("proposal", &proposal.id.0, &proposal)?],
            vec![Self::absent("proposal", &proposal.id.0)],
        )
    }

    fn proposals(&self, work_id: &WorkId) -> Result<Vec<WorkProposal>> {
        let mut values: Vec<WorkProposal> = self.scan("proposal")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn load_proposal(&self, id: &ProposalId) -> Result<Option<WorkProposal>> {
        self.get("proposal", &id.0)
    }

    fn set_proposal_status(&self, id: &ProposalId, status: ProposalStatus) -> Result<WorkProposal> {
        let version = self.version("proposal", &id.0)?;
        let mut proposal = self
            .load_proposal(id)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "proposal".into(),
                id: id.0.clone(),
            })?;
        let allowed = matches!(
            (&proposal.status, &status),
            (ProposalStatus::Draft, ProposalStatus::Proposed)
                | (ProposalStatus::Draft, ProposalStatus::Superseded)
                | (ProposalStatus::Proposed, ProposalStatus::Superseded)
        );
        if !allowed {
            return Err(StateError::InvalidEntity(
                "proposal transition is not allowed".into(),
            ));
        }
        proposal.status = status;
        proposal.updated_at = Utc::now();
        self.atomic(
            vec![Self::mutation("proposal", &proposal.id.0, &proposal)?],
            vec![Self::at_version("proposal", &proposal.id.0, version)],
        )?;
        Ok(proposal)
    }

    fn review_proposal(
        &self,
        review: ProposalReview,
    ) -> Result<(WorkProposal, Option<WorkDecision>)> {
        let version = self.version("proposal", &review.proposal_id.0)?;
        let mut proposal =
            self.load_proposal(&review.proposal_id)?
                .ok_or_else(|| StateError::NotFound {
                    entity_type: "proposal".into(),
                    id: review.proposal_id.0.clone(),
                })?;
        if proposal.status != ProposalStatus::Proposed {
            return Err(StateError::InvalidEntity(
                "only a proposed proposal can be reviewed".into(),
            ));
        }
        self.validate_participant(&proposal.work_id, &review.reviewed_by)?;
        self.validate_origin(&proposal.work_id, &review.reviewed_by, &review.origin)?;
        proposal.status = match review.outcome {
            ReviewOutcome::Approve => ProposalStatus::Approved,
            ReviewOutcome::Reject => ProposalStatus::Rejected,
            ReviewOutcome::RequestChanges => ProposalStatus::Proposed,
        };
        proposal.updated_at = review.created_at;
        let decision = (review.outcome == ReviewOutcome::Approve).then(|| WorkDecision {
            id: Default::default(),
            work_id: proposal.work_id.clone(),
            statement: proposal.statement.clone(),
            rationale: proposal.rationale.clone(),
            decided_by: review.reviewed_by.clone(),
            created_at: review.created_at,
            proposal_id: Some(proposal.id.clone()),
            review_id: Some(review.id.clone()),
        });
        let mut mutations = vec![
            Self::mutation("proposal", &proposal.id.0, &proposal)?,
            Self::mutation("proposal-review", &review.id.0, &review)?,
        ];
        let mut preconditions = vec![
            Self::at_version("proposal", &proposal.id.0, version),
            Self::absent("proposal-review", &review.id.0),
        ];
        if let Some(decision) = &decision {
            mutations.push(Self::mutation("decision", &decision.id.0, decision)?);
            preconditions.push(Self::absent("decision", &decision.id.0));
        }
        self.atomic(mutations, preconditions)?;
        Ok((proposal, decision))
    }

    fn reviews(&self, proposal_id: &ProposalId) -> Result<Vec<ProposalReview>> {
        let mut values: Vec<ProposalReview> = self.scan("proposal-review")?;
        values.retain(|value| value.proposal_id == *proposal_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn load_review(&self, id: &ReviewId) -> Result<Option<ProposalReview>> {
        self.get("proposal-review", &id.0)
    }

    fn start_execution(&self, execution: WorkExecution) -> Result<WorkExecution> {
        if let Some(existing) = self.execution_for_assignment(&execution.assignment_id)? {
            return Ok(existing);
        }
        if execution.status != ExecutionStatus::Started
            || execution.completed_at.is_some()
            || execution.result_id.is_some()
            || execution.failure.is_some()
            || execution.provider.trim().is_empty()
        {
            return Err(StateError::InvalidEntity(
                "new execution must be a valid started execution".into(),
            ));
        }
        let assignment_version = self.version("assignment", &execution.assignment_id.0)?;
        let mut assignment = self
            .get::<WorkAssignment>("assignment", &execution.assignment_id.0)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "assignment".into(),
                id: execution.assignment_id.0.clone(),
            })?;
        if assignment.status != AssignmentStatus::Pending
            || assignment.work_id != execution.work_id
            || assignment.to_participant_id != execution.participant_id
        {
            return Err(StateError::InvalidEntity(
                "assignment is not authorized to start this execution".into(),
            ));
        }
        self.validate_participant(&execution.work_id, &execution.participant_id)?;
        assignment.status = AssignmentStatus::Active;
        self.atomic(
            vec![
                Self::mutation("assignment", &assignment.id.0, &assignment)?,
                Self::mutation("execution", &execution.id.0, &execution)?,
            ],
            vec![
                Self::at_version("assignment", &assignment.id.0, assignment_version),
                Self::absent("execution", &execution.id.0),
            ],
        )?;
        Ok(execution)
    }

    fn heartbeat_execution(
        &self,
        id: &ExecutionId,
        at: chrono::DateTime<Utc>,
    ) -> Result<WorkExecution> {
        let version = self.version("execution", &id.0)?;
        let mut execution = self
            .load_execution(id)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "execution".into(),
                id: id.0.clone(),
            })?;
        if execution.status != ExecutionStatus::Started {
            return Err(StateError::InvalidEntity(
                "only a started execution accepts heartbeats".into(),
            ));
        }
        execution.heartbeat_at = Some(at);
        execution.updated_at = at;
        self.atomic(
            vec![Self::mutation("execution", &execution.id.0, &execution)?],
            vec![Self::at_version("execution", &execution.id.0, version)],
        )?;
        Ok(execution)
    }

    fn finish_execution(
        &self,
        mut execution: WorkExecution,
        result: Option<ParticipantResult>,
        turn: Option<WorkTurn>,
        artifacts: Vec<WorkArtifact>,
    ) -> Result<WorkExecution> {
        let existing = self
            .load_execution(&execution.id)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "execution".into(),
                id: execution.id.0.clone(),
            })?;
        if existing.status != ExecutionStatus::Started {
            if existing.status == execution.status {
                return Ok(existing);
            }
            return Err(StateError::InvalidEntity(
                "execution is already terminal".into(),
            ));
        }
        if !matches!(
            execution.status,
            ExecutionStatus::Completed | ExecutionStatus::Failed | ExecutionStatus::Cancelled
        ) || execution.assignment_id != existing.assignment_id
            || execution.work_id != existing.work_id
            || execution.participant_id != existing.participant_id
            || execution.provider != existing.provider
            || existing.provider_execution_id.is_some()
                && execution.provider_execution_id != existing.provider_execution_id
        {
            return Err(StateError::InvalidEntity(
                "execution terminal provenance does not match".into(),
            ));
        }
        let execution_version = self.version("execution", &execution.id.0)?;
        let assignment_version = self.version("assignment", &execution.assignment_id.0)?;
        let mut assignment = self
            .get::<WorkAssignment>("assignment", &execution.assignment_id.0)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "assignment".into(),
                id: execution.assignment_id.0.clone(),
            })?;
        if assignment.status != AssignmentStatus::Active {
            return Err(StateError::InvalidEntity(
                "execution assignment is not active".into(),
            ));
        }
        let needs_result = matches!(
            execution.status,
            ExecutionStatus::Completed | ExecutionStatus::Failed
        );
        if needs_result != result.is_some() || needs_result != turn.is_some() {
            return Err(StateError::InvalidEntity(
                "completed and failed executions require one result and turn".into(),
            ));
        }
        if let (Some(result), Some(turn)) = (&result, &turn) {
            if result.work_id != execution.work_id
                || result.assignment_id != execution.assignment_id
                || result.participant_id != execution.participant_id
                || result.execution_id != execution.id.0
                || turn.work_id != execution.work_id
                || turn.participant_id != execution.participant_id
                || turn.assignment_id.as_ref() != Some(&execution.assignment_id)
                || turn.execution_id.as_deref() != Some(execution.id.0.as_str())
            {
                return Err(StateError::InvalidEntity(
                    "execution result provenance does not match".into(),
                ));
            }
            validate_contribution(&turn.content)?;
            if result
                .output
                .as_ref()
                .is_some_and(|output| output.len() > CONTEXT_TEXT_LIMIT)
            {
                return Err(StateError::InvalidEntity(
                    "execution result exceeds the context text limit".into(),
                ));
            }
            execution.result_id = Some(result.id.clone());
        }
        if artifacts.iter().any(|artifact| {
            artifact.work_id != execution.work_id || artifact.created_by != execution.participant_id
        }) {
            return Err(StateError::InvalidEntity(
                "execution artifact provenance does not match".into(),
            ));
        }
        assignment.status = match execution.status {
            ExecutionStatus::Completed => AssignmentStatus::Completed,
            ExecutionStatus::Failed => AssignmentStatus::Failed,
            ExecutionStatus::Cancelled => AssignmentStatus::Cancelled,
            ExecutionStatus::Started => unreachable!(),
        };
        let mut mutations = vec![
            Self::mutation("assignment", &assignment.id.0, &assignment)?,
            Self::mutation("execution", &execution.id.0, &execution)?,
        ];
        let mut preconditions = vec![
            Self::at_version("assignment", &assignment.id.0, assignment_version),
            Self::at_version("execution", &execution.id.0, execution_version),
        ];
        if let Some(result) = result {
            mutations.push(Self::mutation("result", &result.id, &result)?);
            preconditions.push(Self::absent("result", &result.id));
        }
        if let Some(turn) = turn {
            mutations.push(Self::mutation("turn", &turn.id.0, &turn)?);
            preconditions.push(Self::absent("turn", &turn.id.0));
        }
        for artifact in artifacts {
            mutations.push(Self::mutation("artifact", &artifact.id.0, &artifact)?);
            preconditions.push(Self::absent("artifact", &artifact.id.0));
        }
        self.atomic(mutations, preconditions)?;
        Ok(execution)
    }

    fn executions(&self, work_id: &WorkId) -> Result<Vec<WorkExecution>> {
        let mut values: Vec<WorkExecution> = self.scan("execution")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn load_execution(&self, id: &ExecutionId) -> Result<Option<WorkExecution>> {
        self.get("execution", &id.0)
    }

    fn execution_for_assignment(&self, id: &AssignmentId) -> Result<Option<WorkExecution>> {
        Ok(self
            .scan::<WorkExecution>("execution")?
            .into_iter()
            .find(|execution| execution.assignment_id == *id))
    }

    fn execution_reviews(&self, work_id: &WorkId) -> Result<Vec<ExecutionReview>> {
        let mut values: Vec<ExecutionReview> = self.scan("execution-review")?;
        values.retain(|value| value.work_id == *work_id);
        values.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(values)
    }

    fn load_execution_review(&self, id: &ExecutionReviewId) -> Result<Option<ExecutionReview>> {
        self.get("execution-review", &id.0)
    }

    fn current_revision(&self) -> Result<u64> {
        self.db
            .current_revision()
            .map_err(|error| StateError::FeltDbError(error.to_string()))
    }

    fn accept_contribution(
        &self,
        contribution: WorkContribution,
    ) -> Result<ContributionAcceptance> {
        if contribution.context_version != WORK_CONTEXT_VERSION {
            return Err(StateError::InvalidEntity(format!(
                "unsupported Work context version {}",
                contribution.context_version
            )));
        }
        self.require_work(&contribution.work_id)?;
        let participant = self
            .load_participant(&contribution.participant_id)?
            .filter(|participant| participant.work_id == contribution.work_id)
            .ok_or_else(|| {
                StateError::InvalidEntity("participant does not belong to Work".into())
            })?;
        self.validate_origin(
            &contribution.work_id,
            &contribution.participant_id,
            &contribution.source,
        )?;
        validate_contribution(&contribution.content)?;
        if let Some(proposal_id) = &contribution.proposal_id
            && self
                .load_proposal(proposal_id)?
                .is_none_or(|proposal| proposal.work_id != contribution.work_id)
        {
            return Err(StateError::InvalidEntity(
                "proposal reference does not belong to Work".into(),
            ));
        }
        if let Some(assignment_id) = &contribution.assignment_id
            && self
                .load_assignment(assignment_id)?
                .is_none_or(|assignment| assignment.work_id != contribution.work_id)
        {
            return Err(StateError::InvalidEntity(
                "assignment reference does not belong to Work".into(),
            ));
        }
        if let Some(execution_id) = &contribution.execution_id {
            let execution = self.load_execution(execution_id)?.ok_or_else(|| {
                StateError::InvalidEntity("execution reference does not belong to Work".into())
            })?;
            if execution.work_id != contribution.work_id
                || contribution
                    .assignment_id
                    .as_ref()
                    .is_some_and(|id| *id != execution.assignment_id)
            {
                return Err(StateError::InvalidEntity(
                    "execution reference does not match Work and Assignment".into(),
                ));
            }
        }
        let expected = Some(contribution.based_on_revision);
        match contribution.kind {
            ContributionKind::Message => {
                let turn = WorkTurn {
                    id: TurnId::new(),
                    work_id: contribution.work_id,
                    participant_id: contribution.participant_id,
                    kind: TurnKind::Message,
                    content: contribution.content,
                    created_at: contribution.created_at,
                    assignment_id: contribution.assignment_id,
                    execution_id: contribution.execution_id.map(|id| id.0),
                    origin: contribution.source,
                };
                let id = turn.id.clone();
                self.atomic_at(
                    expected,
                    vec![Self::mutation("turn", &id.0, &turn)?],
                    vec![Self::absent("turn", &id.0)],
                )?;
                Ok(ContributionAcceptance::Turn(id))
            }
            ContributionKind::Proposal => {
                if !participant.capabilities.can_propose {
                    return Err(StateError::InvalidEntity(
                        "participant cannot propose".into(),
                    ));
                }
                let title = contribution.title.ok_or_else(|| {
                    StateError::InvalidEntity("proposal contribution needs a title".into())
                })?;
                if title.trim().is_empty() {
                    return Err(StateError::InvalidEntity(
                        "proposal contribution title cannot be empty".into(),
                    ));
                }
                let proposal = WorkProposal {
                    id: ProposalId::new(),
                    work_id: contribution.work_id,
                    proposed_by: contribution.participant_id,
                    title,
                    statement: contribution.content,
                    rationale: contribution.rationale,
                    status: ProposalStatus::Proposed,
                    origin: contribution.source,
                    created_at: contribution.created_at,
                    updated_at: contribution.created_at,
                };
                self.atomic_at(
                    expected,
                    vec![Self::mutation("proposal", &proposal.id.0, &proposal)?],
                    vec![Self::absent("proposal", &proposal.id.0)],
                )?;
                Ok(ContributionAcceptance::Proposal(proposal.id))
            }
            ContributionKind::Review => {
                if !participant.capabilities.can_review {
                    return Err(StateError::InvalidEntity(
                        "participant cannot review".into(),
                    ));
                }
                match (contribution.proposal_id, contribution.execution_id) {
                    (Some(proposal_id), None) => {
                        let version = self.version("proposal", &proposal_id.0)?;
                        let mut proposal = self.load_proposal(&proposal_id)?.ok_or_else(|| {
                            StateError::NotFound {
                                entity_type: "proposal".into(),
                                id: proposal_id.0.clone(),
                            }
                        })?;
                        if proposal.work_id != contribution.work_id
                            || proposal.status != ProposalStatus::Proposed
                        {
                            return Err(StateError::InvalidEntity(
                                "proposal review reference is not reviewable".into(),
                            ));
                        }
                        let outcome = contribution.review_outcome.ok_or_else(|| {
                            StateError::InvalidEntity("proposal review needs an outcome".into())
                        })?;
                        if outcome == ReviewOutcome::Approve && !participant.capabilities.can_decide
                        {
                            return Err(StateError::InvalidEntity(
                                "participant cannot approve a proposal".into(),
                            ));
                        }
                        let review = ProposalReview {
                            id: ReviewId::new(),
                            proposal_id: proposal.id.clone(),
                            reviewed_by: contribution.participant_id,
                            outcome,
                            comment: Some(contribution.content),
                            origin: contribution.source,
                            created_at: contribution.created_at,
                        };
                        proposal.status = match outcome {
                            ReviewOutcome::Approve => ProposalStatus::Approved,
                            ReviewOutcome::Reject => ProposalStatus::Rejected,
                            ReviewOutcome::RequestChanges => ProposalStatus::Proposed,
                        };
                        proposal.updated_at = review.created_at;
                        let decision = (outcome == ReviewOutcome::Approve).then(|| WorkDecision {
                            id: Default::default(),
                            work_id: proposal.work_id.clone(),
                            statement: proposal.statement.clone(),
                            rationale: proposal.rationale.clone(),
                            decided_by: review.reviewed_by.clone(),
                            created_at: review.created_at,
                            proposal_id: Some(proposal.id.clone()),
                            review_id: Some(review.id.clone()),
                        });
                        let mut mutations = vec![
                            Self::mutation("proposal", &proposal.id.0, &proposal)?,
                            Self::mutation("proposal-review", &review.id.0, &review)?,
                        ];
                        let mut preconditions = vec![
                            Self::at_version("proposal", &proposal.id.0, version),
                            Self::absent("proposal-review", &review.id.0),
                        ];
                        if let Some(decision) = decision {
                            mutations.push(Self::mutation("decision", &decision.id.0, &decision)?);
                            preconditions.push(Self::absent("decision", &decision.id.0));
                        }
                        self.atomic_at(expected, mutations, preconditions)?;
                        Ok(ContributionAcceptance::ProposalReview(review.id))
                    }
                    (None, Some(execution_id)) => {
                        let execution = self.load_execution(&execution_id)?.ok_or_else(|| {
                            StateError::NotFound {
                                entity_type: "execution".into(),
                                id: execution_id.0.clone(),
                            }
                        })?;
                        let assignment_id = contribution.assignment_id.ok_or_else(|| {
                            StateError::InvalidEntity(
                                "execution review needs its assignment".into(),
                            )
                        })?;
                        if execution.work_id != contribution.work_id
                            || execution.assignment_id != assignment_id
                            || execution.status == ExecutionStatus::Started
                        {
                            return Err(StateError::InvalidEntity(
                                "execution review references do not match a terminal execution"
                                    .into(),
                            ));
                        }
                        let review = ExecutionReview {
                            id: ExecutionReviewId::new(),
                            work_id: contribution.work_id,
                            execution_id,
                            assignment_id,
                            reviewed_by: contribution.participant_id,
                            content: contribution.content,
                            origin: contribution.source,
                            context_revision: contribution.based_on_revision,
                            created_at: contribution.created_at,
                        };
                        self.atomic_at(
                            expected,
                            vec![Self::mutation("execution-review", &review.id.0, &review)?],
                            vec![Self::absent("execution-review", &review.id.0)],
                        )?;
                        Ok(ContributionAcceptance::ExecutionReview(review.id))
                    }
                    _ => Err(StateError::InvalidEntity(
                        "review must reference exactly one proposal or execution".into(),
                    )),
                }
            }
            ContributionKind::Decision => {
                if !participant.capabilities.can_decide {
                    return Err(StateError::InvalidEntity(
                        "participant cannot decide".into(),
                    ));
                }
                let decision = WorkDecision {
                    id: Default::default(),
                    work_id: contribution.work_id,
                    statement: contribution.content,
                    rationale: contribution.rationale,
                    decided_by: contribution.participant_id,
                    created_at: contribution.created_at,
                    proposal_id: contribution.proposal_id,
                    review_id: None,
                };
                self.atomic_at(
                    expected,
                    vec![Self::mutation("decision", &decision.id.0, &decision)?],
                    vec![Self::absent("decision", &decision.id.0)],
                )?;
                Ok(ContributionAcceptance::Decision(decision.id))
            }
            ContributionKind::ExecutionReport | ContributionKind::ArtifactReport => {
                Err(StateError::InvalidEntity(
                    "execution and artifact reports use the canonical execution transition".into(),
                ))
            }
        }
    }
}

fn validate_contribution(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(StateError::InvalidEntity(
            "contribution cannot be empty".into(),
        ));
    }
    if value.len() > CONTEXT_TEXT_LIMIT {
        return Err(StateError::InvalidEntity(format!(
            "contribution exceeds {CONTEXT_TEXT_LIMIT} bytes"
        )));
    }
    Ok(())
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
    use crate::{
        ArtifactKind, ConversationProvider, ConversationRef, DecisionId, ParticipantKind, TurnKind,
    };
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
    fn same_process_work_stores_share_current_state() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let gui = FeltDbWorkStore::open(&path).unwrap();
        let (work, _, _) = work_with_participants(&gui);
        let cli = FeltDbWorkStore::open(&path).unwrap();
        assert_eq!(cli.load_work(&work.id).unwrap(), Some(work.clone()));
        cli.set_work_status(&work.id, WorkStatus::Completed)
            .unwrap();
        assert_eq!(
            gui.load_work(&work.id).unwrap().unwrap().status,
            WorkStatus::Completed
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
                    origin: TurnOrigin::Local,
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
            proposal_id: None,
        };
        store.add_assignment(assignment.clone()).unwrap();
        let decision = WorkDecision {
            id: DecisionId::new(),
            work_id: work.id.clone(),
            statement: "Use FeltDB".into(),
            rationale: Some("One authority".into()),
            decided_by: human.id.clone(),
            created_at: Utc::now(),
            proposal_id: None,
            review_id: None,
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
    }

    #[test]
    fn external_conversation_and_provenance_persist_after_reopen() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let (work_id, conversation_id, turn_id, reference) = {
            let store = FeltDbWorkStore::open(&path).unwrap();
            let (work, _, agent) = work_with_participants(&store);
            let reference = ConversationRef {
                id: "chat-42".into(),
                provider: ConversationProvider::ChatGpt,
                title: Some("Architecture review".into()),
            };
            let conversation = WorkConversation {
                id: ConversationId::new(),
                work_id: work.id.clone(),
                conversation: reference.clone(),
                participant_id: agent.id.clone(),
                label: Some("ChatGPT review".into()),
                created_at: Utc::now(),
            };
            let turn = WorkTurn {
                id: TurnId::new(),
                work_id: work.id.clone(),
                participant_id: agent.id,
                kind: TurnKind::Review,
                content: "Keep the boundary provider-neutral.".into(),
                created_at: Utc::now(),
                assignment_id: None,
                execution_id: None,
                origin: TurnOrigin::ExternalConversation(reference.clone()),
            };
            store
                .import_contribution(conversation.clone(), turn.clone())
                .unwrap();
            (work.id, conversation.id, turn.id, reference)
        };
        let store = FeltDbWorkStore::open(path).unwrap();
        assert_eq!(
            store
                .load_conversation(&conversation_id)
                .unwrap()
                .unwrap()
                .conversation,
            reference
        );
        assert_eq!(
            store.load_turn(&turn_id).unwrap().unwrap().origin,
            TurnOrigin::ExternalConversation(reference)
        );
        assert_eq!(store.context(&work_id).unwrap().conversations.len(), 1);
    }

    #[test]
    fn invalid_import_writes_neither_conversation_nor_turn() {
        let (store, _directory) = store();
        let (work, _, agent) = work_with_participants(&store);
        let reference = ConversationRef {
            id: "external-1".into(),
            provider: ConversationProvider::Claude,
            title: None,
        };
        let conversation = WorkConversation {
            id: ConversationId::new(),
            work_id: work.id.clone(),
            conversation: reference.clone(),
            participant_id: agent.id.clone(),
            label: None,
            created_at: Utc::now(),
        };
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: work.id,
            participant_id: agent.id,
            kind: TurnKind::Message,
            content: "  ".into(),
            created_at: Utc::now(),
            assignment_id: None,
            execution_id: None,
            origin: TurnOrigin::ExternalConversation(reference),
        };
        assert!(
            store
                .import_contribution(conversation.clone(), turn.clone())
                .is_err()
        );
        assert!(store.load_conversation(&conversation.id).unwrap().is_none());
        assert!(store.load_turn(&turn.id).unwrap().is_none());
    }

    #[test]
    fn context_bounds_conversation_links() {
        let (store, _directory) = store();
        let (work, _, agent) = work_with_participants(&store);
        for index in 0..=CONTEXT_CONVERSATION_LIMIT {
            let reference = ConversationRef {
                id: format!("external-{index}"),
                provider: ConversationProvider::Other("manual".into()),
                title: None,
            };
            store
                .import_contribution(
                    WorkConversation {
                        id: ConversationId::new(),
                        work_id: work.id.clone(),
                        conversation: reference.clone(),
                        participant_id: agent.id.clone(),
                        label: None,
                        created_at: Utc::now(),
                    },
                    WorkTurn {
                        id: TurnId::new(),
                        work_id: work.id.clone(),
                        participant_id: agent.id.clone(),
                        kind: TurnKind::Message,
                        content: index.to_string(),
                        created_at: Utc::now(),
                        assignment_id: None,
                        execution_id: None,
                        origin: TurnOrigin::ExternalConversation(reference),
                    },
                )
                .unwrap();
        }
        assert_eq!(
            store.context(&work.id).unwrap().conversations.len(),
            CONTEXT_CONVERSATION_LIMIT
        );
    }

    fn proposal(work: &Work, participant: &Participant, status: ProposalStatus) -> WorkProposal {
        let now = Utc::now();
        WorkProposal {
            id: ProposalId::new(),
            work_id: work.id.clone(),
            proposed_by: participant.id.clone(),
            title: "Use the port boundary".into(),
            statement: "Move authorization behind AuthPort.".into(),
            rationale: Some("Keep the domain provider-neutral.".into()),
            status,
            origin: TurnOrigin::Local,
            created_at: now,
            updated_at: now,
        }
    }

    fn review(
        proposal: &WorkProposal,
        participant: &Participant,
        outcome: ReviewOutcome,
    ) -> ProposalReview {
        ProposalReview {
            id: ReviewId::new(),
            proposal_id: proposal.id.clone(),
            reviewed_by: participant.id.clone(),
            outcome,
            comment: Some("Explicit review".into()),
            origin: TurnOrigin::Local,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn proposal_lifecycle_approval_and_restart_preserve_decision_chain() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let (work_id, proposal_id, review_id, decision_id) = {
            let store = FeltDbWorkStore::open(&path).unwrap();
            let (work, human, agent) = work_with_participants(&store);
            let draft = proposal(&work, &agent, ProposalStatus::Draft);
            store.create_proposal(draft.clone()).unwrap();
            let proposed = store
                .set_proposal_status(&draft.id, ProposalStatus::Proposed)
                .unwrap();
            let approval = review(&proposed, &human, ReviewOutcome::Approve);
            let (approved, decision) = store.review_proposal(approval.clone()).unwrap();
            let decision = decision.unwrap();
            assert_eq!(approved.status, ProposalStatus::Approved);
            assert_eq!(decision.proposal_id.as_ref(), Some(&approved.id));
            assert_eq!(decision.review_id.as_ref(), Some(&approval.id));
            assert_eq!(decision.decided_by, human.id);
            (work.id, approved.id, approval.id, decision.id)
        };
        let store = FeltDbWorkStore::open(path).unwrap();
        assert_eq!(
            store.load_proposal(&proposal_id).unwrap().unwrap().status,
            ProposalStatus::Approved
        );
        assert!(store.load_review(&review_id).unwrap().is_some());
        let decision = store
            .decisions(&work_id)
            .unwrap()
            .into_iter()
            .find(|decision| decision.id == decision_id)
            .unwrap();
        assert_eq!(decision.proposal_id, Some(proposal_id));
        assert_eq!(
            store
                .context(&work_id)
                .unwrap()
                .state
                .approved_proposals
                .len(),
            1
        );
    }

    #[test]
    fn rejection_and_request_changes_have_explicit_semantics() {
        let (store, _directory) = store();
        let (work, human, agent) = work_with_participants(&store);
        let rejected = proposal(&work, &agent, ProposalStatus::Proposed);
        store.create_proposal(rejected.clone()).unwrap();
        assert_eq!(
            store
                .review_proposal(review(&rejected, &human, ReviewOutcome::Reject))
                .unwrap()
                .0
                .status,
            ProposalStatus::Rejected
        );
        let changes = proposal(&work, &agent, ProposalStatus::Proposed);
        store.create_proposal(changes.clone()).unwrap();
        let (changes, decision) = store
            .review_proposal(review(&changes, &human, ReviewOutcome::RequestChanges))
            .unwrap();
        assert_eq!(changes.status, ProposalStatus::Proposed);
        assert!(decision.is_none());
        assert_eq!(
            store.reviews(&changes.id).unwrap()[0].outcome,
            ReviewOutcome::RequestChanges
        );
    }

    #[test]
    fn only_approved_proposals_can_be_assigned() {
        let (store, _directory) = store();
        let (work, human, agent) = work_with_participants(&store);
        let proposed = proposal(&work, &human, ProposalStatus::Proposed);
        store.create_proposal(proposed.clone()).unwrap();
        let assignment = WorkAssignment {
            id: AssignmentId::new(),
            work_id: work.id.clone(),
            from_participant_id: human.id.clone(),
            to_participant_id: agent.id.clone(),
            instruction: "Implement the proposal".into(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
            proposal_id: Some(proposed.id.clone()),
        };
        assert!(store.add_assignment(assignment.clone()).is_err());
        store
            .review_proposal(review(&proposed, &human, ReviewOutcome::Approve))
            .unwrap();
        store.add_assignment(assignment.clone()).unwrap();
        assert_eq!(
            store.assignments(&work.id).unwrap()[0].proposal_id,
            Some(proposed.id)
        );
    }

    #[test]
    fn failed_approval_transaction_leaves_no_partial_state() {
        let (store, _directory) = store();
        let (work, human, agent) = work_with_participants(&store);
        let proposed = proposal(&work, &agent, ProposalStatus::Proposed);
        store.create_proposal(proposed.clone()).unwrap();
        let approval = review(&proposed, &human, ReviewOutcome::Approve);
        store
            .insert("proposal-review", &approval.id.0, &approval)
            .unwrap();
        assert!(store.review_proposal(approval.clone()).is_err());
        assert_eq!(
            store.load_proposal(&proposed.id).unwrap().unwrap().status,
            ProposalStatus::Proposed
        );
        assert!(store.decisions(&work.id).unwrap().is_empty());
        assert_eq!(store.reviews(&proposed.id).unwrap(), vec![approval]);
    }

    #[test]
    fn external_proposal_to_human_approval_to_agent_assignment_preserves_provenance() {
        let (store, _directory) = store();
        let (work, human, agent) = work_with_participants(&store);
        let chatgpt = Participant::new(work.id.clone(), ParticipantKind::Agent, "ChatGPT".into());
        store.add_participant(chatgpt.clone()).unwrap();
        let reference = ConversationRef {
            id: "chatgpt-architecture".into(),
            provider: ConversationProvider::ChatGpt,
            title: Some("Architecture".into()),
        };
        store
            .import_contribution(
                WorkConversation {
                    id: ConversationId::new(),
                    work_id: work.id.clone(),
                    conversation: reference.clone(),
                    participant_id: chatgpt.id.clone(),
                    label: Some("ChatGPT architecture".into()),
                    created_at: Utc::now(),
                },
                WorkTurn {
                    id: TurnId::new(),
                    work_id: work.id.clone(),
                    participant_id: chatgpt.id.clone(),
                    kind: TurnKind::Analysis,
                    content: "Use an explicit port.".into(),
                    created_at: Utc::now(),
                    assignment_id: None,
                    execution_id: None,
                    origin: TurnOrigin::ExternalConversation(reference.clone()),
                },
            )
            .unwrap();
        let mut proposed = proposal(&work, &chatgpt, ProposalStatus::Proposed);
        proposed.origin = TurnOrigin::ExternalConversation(reference.clone());
        store.create_proposal(proposed.clone()).unwrap();
        let approval = review(&proposed, &human, ReviewOutcome::Approve);
        let (_, decision) = store.review_proposal(approval.clone()).unwrap();
        let assignment = WorkAssignment {
            id: AssignmentId::new(),
            work_id: work.id.clone(),
            from_participant_id: human.id.clone(),
            to_participant_id: agent.id,
            instruction: "Implement the explicit port.".into(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
            proposal_id: Some(proposed.id.clone()),
        };
        store.add_assignment(assignment.clone()).unwrap();
        let context = store.context(&work.id).unwrap();
        assert_eq!(
            context.proposals[0].origin,
            TurnOrigin::ExternalConversation(reference)
        );
        assert_eq!(context.reviews[0].reviewed_by, human.id);
        assert_eq!(decision.unwrap().review_id, Some(approval.id));
        assert_eq!(context.active_assignments[0].proposal_id, Some(proposed.id));
    }

    fn assigned_execution(
        store: &FeltDbWorkStore,
    ) -> (Work, Participant, WorkAssignment, WorkExecution) {
        let (work, human, agent) = work_with_participants(store);
        let assignment = WorkAssignment {
            id: AssignmentId::new(),
            work_id: work.id.clone(),
            from_participant_id: human.id,
            to_participant_id: agent.id.clone(),
            instruction: "Perform authorized work".into(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
            proposal_id: None,
        };
        store.add_assignment(assignment.clone()).unwrap();
        let now = Utc::now();
        let execution = WorkExecution {
            id: ExecutionId::new(),
            work_id: work.id.clone(),
            assignment_id: assignment.id.clone(),
            participant_id: agent.id.clone(),
            provider: "test-provider".into(),
            provider_execution_id: Some("provider-42".into()),
            status: ExecutionStatus::Started,
            started_at: now,
            completed_at: None,
            heartbeat_at: Some(now),
            result_id: None,
            failure: None,
            created_at: now,
            updated_at: now,
        };
        (work, agent, assignment, execution)
    }

    fn terminal_records(
        work: &Work,
        participant: &Participant,
        execution: &WorkExecution,
        exit_status: Option<i32>,
    ) -> (ParticipantResult, WorkTurn) {
        let now = Utc::now();
        (
            ParticipantResult {
                id: Uuid::new_v4().to_string(),
                work_id: work.id.clone(),
                participant_id: participant.id.clone(),
                assignment_id: execution.assignment_id.clone(),
                execution_id: execution.id.0.clone(),
                exit_status,
                summary: Some("outcome".into()),
                output: Some("bounded outcome".into()),
                created_at: now,
            },
            WorkTurn {
                id: TurnId::new(),
                work_id: work.id.clone(),
                participant_id: participant.id.clone(),
                kind: TurnKind::Implementation,
                content: "bounded outcome".into(),
                created_at: now,
                assignment_id: Some(execution.assignment_id.clone()),
                execution_id: Some(execution.id.0.clone()),
                origin: TurnOrigin::Local,
            },
        )
    }

    #[test]
    fn execution_start_and_completion_are_fenced_idempotent_and_durable() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let (work_id, assignment_id, execution_id, result_id) = {
            let store = FeltDbWorkStore::open(&path).unwrap();
            let (work, agent, assignment, execution) = assigned_execution(&store);
            let started = store.start_execution(execution.clone()).unwrap();
            let duplicate = store
                .start_execution(WorkExecution {
                    id: ExecutionId::new(),
                    ..execution.clone()
                })
                .unwrap();
            assert_eq!(duplicate.id, started.id);
            assert_eq!(
                store
                    .load_assignment(&assignment.id)
                    .unwrap()
                    .unwrap()
                    .status,
                AssignmentStatus::Active
            );
            let (result, turn) = terminal_records(&work, &agent, &started, Some(0));
            let result_id = result.id.clone();
            let mut completed = started.clone();
            completed.status = ExecutionStatus::Completed;
            completed.completed_at = Some(Utc::now());
            completed.updated_at = Utc::now();
            let completed = store
                .finish_execution(completed.clone(), Some(result), Some(turn), Vec::new())
                .unwrap();
            assert_eq!(
                store
                    .finish_execution(completed.clone(), None, None, Vec::new())
                    .unwrap(),
                completed
            );
            assert_eq!(
                store
                    .load_assignment(&assignment.id)
                    .unwrap()
                    .unwrap()
                    .status,
                AssignmentStatus::Completed
            );
            (work.id, assignment.id, completed.id, result_id)
        };
        let store = FeltDbWorkStore::open(path).unwrap();
        let execution = store.load_execution(&execution_id).unwrap().unwrap();
        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.result_id.as_deref(), Some(result_id.as_str()));
        assert!(store.load_result(&result_id).unwrap().is_some());
        let context = store.context(&work_id).unwrap();
        assert_eq!(context.executions.len(), 1);
        assert_eq!(context.results[0].id, result_id);
        assert_eq!(
            store
                .load_assignment(&assignment_id)
                .unwrap()
                .unwrap()
                .status,
            AssignmentStatus::Completed
        );
    }

    #[test]
    fn execution_failure_cancellation_and_invalid_transitions_are_explicit() {
        let (store, _directory) = store();
        let (work, agent, _, execution) = assigned_execution(&store);
        let started = store.start_execution(execution).unwrap();
        let (result, turn) = terminal_records(&work, &agent, &started, Some(1));
        let mut failed = started.clone();
        failed.status = ExecutionStatus::Failed;
        failed.failure = Some("provider failed".into());
        failed.completed_at = Some(Utc::now());
        failed.updated_at = Utc::now();
        store
            .finish_execution(failed.clone(), Some(result), Some(turn), Vec::new())
            .unwrap();
        let mut contradictory = failed.clone();
        contradictory.status = ExecutionStatus::Completed;
        assert!(
            store
                .finish_execution(contradictory, None, None, Vec::new())
                .is_err()
        );

        let (_, _, assignment, execution) = assigned_execution(&store);
        let mut cancelled = store.start_execution(execution).unwrap();
        cancelled.status = ExecutionStatus::Cancelled;
        cancelled.failure = Some("cancelled by human".into());
        cancelled.completed_at = Some(Utc::now());
        cancelled.updated_at = Utc::now();
        store
            .finish_execution(cancelled.clone(), None, None, Vec::new())
            .unwrap();
        assert_eq!(
            store
                .load_assignment(&assignment.id)
                .unwrap()
                .unwrap()
                .status,
            AssignmentStatus::Cancelled
        );
        assert_eq!(
            store
                .finish_execution(cancelled.clone(), None, None, Vec::new())
                .unwrap()
                .status,
            ExecutionStatus::Cancelled
        );
    }

    #[test]
    fn failed_execution_transaction_preserves_started_state() {
        let (store, _directory) = store();
        let (work, agent, assignment, execution) = assigned_execution(&store);
        let started = store.start_execution(execution).unwrap();
        let (result, turn) = terminal_records(&work, &agent, &started, Some(0));
        store.insert("result", &result.id, &result).unwrap();
        let mut completed = started.clone();
        completed.status = ExecutionStatus::Completed;
        completed.completed_at = Some(Utc::now());
        completed.updated_at = Utc::now();
        assert!(
            store
                .finish_execution(completed, Some(result), Some(turn.clone()), Vec::new())
                .is_err()
        );
        assert_eq!(
            store.load_execution(&started.id).unwrap().unwrap().status,
            ExecutionStatus::Started
        );
        assert_eq!(
            store
                .load_assignment(&assignment.id)
                .unwrap()
                .unwrap()
                .status,
            AssignmentStatus::Active
        );
        assert!(store.load_turn(&turn.id).unwrap().is_none());
    }

    #[test]
    fn execution_projection_derives_staleness_and_terminal_groups() {
        let (store, _directory) = store();
        let (work, _, _, mut execution) = assigned_execution(&store);
        execution.started_at = Utc::now() - chrono::Duration::minutes(20);
        execution.heartbeat_at = Some(execution.started_at);
        execution.created_at = execution.started_at;
        execution.updated_at = execution.started_at;
        store.start_execution(execution).unwrap();
        let context = store.context(&work.id).unwrap();
        assert!(context.state.active_executions.is_empty());
        assert_eq!(context.state.stale_executions.len(), 1);
        let first = serde_json::to_vec(&context).unwrap();
        let second = serde_json::to_vec(&store.context(&work.id).unwrap()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn concurrent_execution_starts_create_one_canonical_record() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let first = FeltDbWorkStore::open(&path).unwrap();
        let (_, _, assignment, execution) = assigned_execution(&first);
        let second = FeltDbWorkStore::open(&path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let run = |store: FeltDbWorkStore,
                   mut execution: WorkExecution,
                   barrier: std::sync::Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                execution.id = ExecutionId::new();
                barrier.wait();
                store.start_execution(execution)
            })
        };
        let left = run(first, execution.clone(), barrier.clone());
        let right = run(second, execution, barrier.clone());
        barrier.wait();
        let left = left.join().unwrap();
        let right = right.join().unwrap();
        assert!(left.is_ok() || right.is_ok());
        let store = FeltDbWorkStore::open(path).unwrap();
        assert_eq!(store.executions(&assignment.work_id).unwrap().len(), 1);
        assert_eq!(
            store
                .load_assignment(&assignment.id)
                .unwrap()
                .unwrap()
                .status,
            AssignmentStatus::Active
        );
    }

    #[test]
    fn concurrent_completion_and_cancellation_choose_one_terminal_outcome() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let setup = FeltDbWorkStore::open(&path).unwrap();
        let (work, agent, assignment, execution) = assigned_execution(&setup);
        let started = setup.start_execution(execution).unwrap();
        let completion_store = FeltDbWorkStore::open(&path).unwrap();
        let cancellation_store = FeltDbWorkStore::open(&path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let completion_barrier = barrier.clone();
        let mut completed = started.clone();
        let completion = std::thread::spawn(move || {
            let (result, turn) = terminal_records(&work, &agent, &completed, Some(0));
            completed.status = ExecutionStatus::Completed;
            completed.completed_at = Some(Utc::now());
            completed.updated_at = Utc::now();
            completion_barrier.wait();
            completion_store.finish_execution(completed, Some(result), Some(turn), Vec::new())
        });
        let cancellation_barrier = barrier.clone();
        let mut cancelled = started.clone();
        let cancellation = std::thread::spawn(move || {
            cancelled.status = ExecutionStatus::Cancelled;
            cancelled.failure = Some("cancelled by human".into());
            cancelled.completed_at = Some(Utc::now());
            cancelled.updated_at = Utc::now();
            cancellation_barrier.wait();
            cancellation_store.finish_execution(cancelled, None, None, Vec::new())
        });
        barrier.wait();
        let completion = completion.join().unwrap();
        let cancellation = cancellation.join().unwrap();
        assert_ne!(completion.is_ok(), cancellation.is_ok());
        let store = FeltDbWorkStore::open(path).unwrap();
        let execution = store.load_execution(&started.id).unwrap().unwrap();
        let assignment = store.load_assignment(&assignment.id).unwrap().unwrap();
        assert!(matches!(
            (execution.status, assignment.status),
            (ExecutionStatus::Completed, AssignmentStatus::Completed)
                | (ExecutionStatus::Cancelled, AssignmentStatus::Cancelled)
        ));
    }

    fn contribution(
        store: &FeltDbWorkStore,
        work: &Work,
        participant: &Participant,
        kind: ContributionKind,
        content: &str,
    ) -> WorkContribution {
        WorkContribution::local(
            work.id.clone(),
            participant.id.clone(),
            store.current_revision().unwrap(),
            kind,
            content.into(),
        )
    }

    #[test]
    fn protocol_version_stale_context_and_malformed_references_write_nothing() {
        let (store, _directory) = store();
        let (work, human, _) = work_with_participants(&store);
        let mut unsupported =
            contribution(&store, &work, &human, ContributionKind::Message, "hello");
        unsupported.context_version = 99;
        assert!(store.accept_contribution(unsupported).is_err());

        let stale_revision = store.current_revision().unwrap();
        store
            .add_turn(WorkTurn {
                id: TurnId::new(),
                work_id: work.id.clone(),
                participant_id: human.id.clone(),
                kind: TurnKind::Message,
                content: "newer state".into(),
                created_at: Utc::now(),
                assignment_id: None,
                execution_id: None,
                origin: TurnOrigin::Local,
            })
            .unwrap();
        let mut stale = contribution(
            &store,
            &work,
            &human,
            ContributionKind::Proposal,
            "stale proposal",
        );
        stale.based_on_revision = stale_revision;
        stale.title = Some("Stale".into());
        assert!(matches!(
            store.accept_contribution(stale),
            Err(StateError::StaleContext { .. })
        ));
        assert!(store.proposals(&work.id).unwrap().is_empty());

        let mut malformed = contribution(&store, &work, &human, ContributionKind::Review, "review");
        malformed.execution_id = Some(ExecutionId::new());
        malformed.assignment_id = Some(AssignmentId::new());
        assert!(store.accept_contribution(malformed).is_err());
        assert!(store.execution_reviews(&work.id).unwrap().is_empty());
    }

    #[test]
    fn external_protocol_proposal_and_execution_review_preserve_the_complete_chain() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("work.db");
        let (work_id, execution_id, review_id) = {
            let store = FeltDbWorkStore::open(&path).unwrap();
            let (work, human, executor) = work_with_participants(&store);
            let chatgpt =
                Participant::new(work.id.clone(), ParticipantKind::Agent, "ChatGPT".into());
            store.add_participant(chatgpt.clone()).unwrap();
            let reference = ConversationRef {
                id: "chatgpt-protocol".into(),
                provider: ConversationProvider::ChatGpt,
                title: Some("Protocol review".into()),
            };
            store
                .import_contribution(
                    WorkConversation {
                        id: ConversationId::new(),
                        work_id: work.id.clone(),
                        conversation: reference.clone(),
                        participant_id: chatgpt.id.clone(),
                        label: Some("ChatGPT protocol".into()),
                        created_at: Utc::now(),
                    },
                    WorkTurn {
                        id: TurnId::new(),
                        work_id: work.id.clone(),
                        participant_id: chatgpt.id.clone(),
                        kind: TurnKind::Analysis,
                        content: "Initial external context".into(),
                        created_at: Utc::now(),
                        assignment_id: None,
                        execution_id: None,
                        origin: TurnOrigin::ExternalConversation(reference.clone()),
                    },
                )
                .unwrap();
            let mut proposal = contribution(
                &store,
                &work,
                &chatgpt,
                ContributionKind::Proposal,
                "Implement the protocol boundary",
            );
            proposal.title = Some("Protocol boundary".into());
            proposal.source = TurnOrigin::ExternalConversation(reference.clone());
            let proposal_id = match store.accept_contribution(proposal).unwrap() {
                ContributionAcceptance::Proposal(id) => id,
                _ => unreachable!(),
            };
            let proposed = store.load_proposal(&proposal_id).unwrap().unwrap();
            let mut approval = contribution(
                &store,
                &work,
                &human,
                ContributionKind::Review,
                "Approved for execution",
            );
            approval.proposal_id = Some(proposed.id.clone());
            approval.review_outcome = Some(ReviewOutcome::Approve);
            assert!(matches!(
                store.accept_contribution(approval).unwrap(),
                ContributionAcceptance::ProposalReview(_)
            ));
            let assignment = WorkAssignment {
                id: AssignmentId::new(),
                work_id: work.id.clone(),
                from_participant_id: human.id.clone(),
                to_participant_id: executor.id.clone(),
                instruction: "Implement approved protocol".into(),
                status: AssignmentStatus::Pending,
                created_at: Utc::now(),
                proposal_id: Some(proposal_id),
            };
            store.add_assignment(assignment.clone()).unwrap();
            let now = Utc::now();
            let mut execution = store
                .start_execution(WorkExecution {
                    id: ExecutionId::new(),
                    work_id: work.id.clone(),
                    assignment_id: assignment.id.clone(),
                    participant_id: executor.id.clone(),
                    provider: "codex".into(),
                    provider_execution_id: Some("codex-run".into()),
                    status: ExecutionStatus::Started,
                    started_at: now,
                    completed_at: None,
                    heartbeat_at: Some(now),
                    result_id: None,
                    failure: None,
                    created_at: now,
                    updated_at: now,
                })
                .unwrap();
            let (result, turn) = terminal_records(&work, &executor, &execution, Some(0));
            execution.status = ExecutionStatus::Completed;
            execution.completed_at = Some(Utc::now());
            execution.updated_at = Utc::now();
            store
                .finish_execution(execution.clone(), Some(result), Some(turn), Vec::new())
                .unwrap();
            let mut execution_review = contribution(
                &store,
                &work,
                &chatgpt,
                ContributionKind::Review,
                "The execution satisfies the approved proposal.",
            );
            execution_review.source = TurnOrigin::ExternalConversation(reference);
            execution_review.assignment_id = Some(assignment.id);
            execution_review.execution_id = Some(execution.id.clone());
            let review_id = match store.accept_contribution(execution_review).unwrap() {
                ContributionAcceptance::ExecutionReview(id) => id,
                _ => unreachable!(),
            };
            (work.id, execution.id, review_id)
        };
        let store = FeltDbWorkStore::open(path).unwrap();
        let context = store.context(&work_id).unwrap();
        assert_eq!(context.execution_reviews[0].id, review_id);
        assert_eq!(context.execution_reviews[0].execution_id, execution_id);
        assert!(matches!(
            context.execution_reviews[0].origin,
            TurnOrigin::ExternalConversation(_)
        ));
        assert_eq!(context.proposals[0].status, ProposalStatus::Approved);
        assert_eq!(context.executions[0].status, ExecutionStatus::Completed);
        assert_eq!(context.results.len(), 1);
    }

    #[test]
    fn protocol_contributions_are_provider_neutral_and_capability_checked() {
        for name in ["ChatGPT", "Claude", "Codex", "Human", "Other"] {
            let (store, _directory) = store();
            let work = Work::new("workspace".into(), "Neutral".into(), None);
            let kind = if name == "Human" {
                ParticipantKind::Human
            } else {
                ParticipantKind::Agent
            };
            let participant = Participant::new(work.id.clone(), kind, name.into());
            store
                .create_work(work.clone(), vec![participant.clone()])
                .unwrap();
            let accepted = store
                .accept_contribution(contribution(
                    &store,
                    &work,
                    &participant,
                    ContributionKind::Message,
                    "provider-neutral contribution",
                ))
                .unwrap();
            assert!(matches!(accepted, ContributionAcceptance::Turn(_)));
        }
    }
}
