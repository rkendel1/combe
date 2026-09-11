use std::path::{Path, PathBuf};

use chrono::Utc;
use feltdb::{AtomicMutation, AtomicPrecondition};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use crate::work_store::{
    CONTEXT_ARTIFACT_LIMIT, CONTEXT_CONVERSATION_LIMIT, CONTEXT_DECISION_LIMIT,
    CONTEXT_PROPOSAL_LIMIT, CONTEXT_REVIEW_LIMIT, CONTEXT_TEXT_LIMIT, CONTEXT_TURN_LIMIT,
};
use crate::{
    ArtifactId, AssignmentId, AssignmentStatus, ConversationId, Participant, ParticipantId,
    ParticipantResult, ProposalId, ProposalReview, ProposalStatus, Result, ReviewId, ReviewOutcome,
    StateError, TurnId, TurnOrigin, Work, WorkArtifact, WorkAssignment, WorkContext,
    WorkConversation, WorkDecision, WorkId, WorkProposal, WorkState, WorkStatus, WorkStore,
    WorkTurn,
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
            .collect::<Vec<_>>();
        let active_assignments = self
            .assignments(work_id)?
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
        };
        Ok(WorkContext {
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
            proposal_id: None,
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
            origin: TurnOrigin::Local,
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
                proposal_id: None,
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
                origin: TurnOrigin::Local,
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
}
