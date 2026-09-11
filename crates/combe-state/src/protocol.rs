use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    AssignmentId, ExecutionId, Participant, ParticipantId, ProposalId, ReviewOutcome, StateError,
    Timestamp, TurnOrigin, WorkId,
};

pub const WORK_CONTEXT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkAction {
    Inspect,
    Propose,
    Review,
    Decide,
    Assign,
    Execute,
    Report,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRequest {
    pub work_id: WorkId,
    pub participant_id: ParticipantId,
    pub context_version: u32,
    pub revision: u64,
    pub requested_action: WorkAction,
}

impl WorkRequest {
    pub fn validate(&self, participant: &Participant) -> crate::Result<()> {
        if self.context_version != WORK_CONTEXT_VERSION {
            return Err(StateError::InvalidEntity(format!(
                "unsupported Work context version {}",
                self.context_version
            )));
        }
        if participant.id != self.participant_id || participant.work_id != self.work_id {
            return Err(StateError::InvalidEntity(
                "Work request participant does not match Work".into(),
            ));
        }
        let allowed = match self.requested_action {
            WorkAction::Inspect => true,
            WorkAction::Propose => participant.capabilities.can_propose,
            WorkAction::Review => participant.capabilities.can_review,
            WorkAction::Decide => participant.capabilities.can_decide,
            WorkAction::Assign => participant.capabilities.can_decide,
            WorkAction::Execute => participant.capabilities.can_execute,
            WorkAction::Report => participant.capabilities.can_execute,
        };
        if !allowed {
            return Err(StateError::InvalidEntity(
                "participant does not have the requested capability".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Message,
    Proposal,
    Review,
    Decision,
    ExecutionReport,
    ArtifactReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkContribution {
    pub work_id: WorkId,
    pub participant_id: ParticipantId,
    pub context_version: u32,
    pub based_on_revision: u64,
    pub source: TurnOrigin,
    pub kind: ContributionKind,
    pub content: String,
    pub title: Option<String>,
    pub rationale: Option<String>,
    pub review_outcome: Option<ReviewOutcome>,
    pub proposal_id: Option<ProposalId>,
    pub assignment_id: Option<AssignmentId>,
    pub execution_id: Option<ExecutionId>,
    pub created_at: Timestamp,
}

impl WorkContribution {
    pub fn local(
        work_id: WorkId,
        participant_id: ParticipantId,
        revision: u64,
        kind: ContributionKind,
        content: String,
    ) -> Self {
        Self {
            work_id,
            participant_id,
            context_version: WORK_CONTEXT_VERSION,
            based_on_revision: revision,
            source: TurnOrigin::Local,
            kind,
            content,
            title: None,
            rationale: None,
            review_outcome: None,
            proposal_id: None,
            assignment_id: None,
            execution_id: None,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionAcceptance {
    Turn(crate::TurnId),
    Proposal(crate::ProposalId),
    ProposalReview(crate::ReviewId),
    ExecutionReview(crate::ExecutionReviewId),
    Decision(crate::DecisionId),
}
