use crate::{
    ArtifactId, AssignmentId, ConversationId, Participant, ParticipantId, ParticipantResult,
    ProposalId, ProposalReview, ProposalStatus, Result, ReviewId, TurnId, Work, WorkArtifact,
    WorkAssignment, WorkContext, WorkConversation, WorkDecision, WorkId, WorkProposal, WorkStatus,
    WorkTurn,
};

pub const CONTEXT_TURN_LIMIT: usize = 50;
pub const CONTEXT_DECISION_LIMIT: usize = 100;
pub const CONTEXT_ARTIFACT_LIMIT: usize = 100;
pub const CONTEXT_TEXT_LIMIT: usize = 16 * 1024;
pub const CONTEXT_CONVERSATION_LIMIT: usize = 100;
pub const CONTEXT_PROPOSAL_LIMIT: usize = 100;
pub const CONTEXT_REVIEW_LIMIT: usize = 100;

pub trait WorkStore: Send + Sync {
    fn create_work(&self, work: Work, participants: Vec<Participant>) -> Result<()>;
    fn load_work(&self, id: &WorkId) -> Result<Option<Work>>;
    fn list_works(&self, workspace_id: Option<&str>) -> Result<Vec<Work>>;
    fn set_work_status(&self, id: &WorkId, status: WorkStatus) -> Result<Work>;
    fn add_participant(&self, participant: Participant) -> Result<()>;
    fn participants(&self, work_id: &WorkId) -> Result<Vec<Participant>>;
    fn add_turn(&self, turn: WorkTurn) -> Result<()>;
    fn turns(&self, work_id: &WorkId) -> Result<Vec<WorkTurn>>;
    fn add_assignment(&self, assignment: WorkAssignment) -> Result<()>;
    fn assignments(&self, work_id: &WorkId) -> Result<Vec<WorkAssignment>>;
    fn set_assignment_status(
        &self,
        id: &AssignmentId,
        status: crate::AssignmentStatus,
    ) -> Result<WorkAssignment>;
    fn record_decision(&self, decision: WorkDecision, turn: Option<WorkTurn>) -> Result<()>;
    fn decisions(&self, work_id: &WorkId) -> Result<Vec<WorkDecision>>;
    fn add_artifact(&self, artifact: WorkArtifact) -> Result<()>;
    fn artifacts(&self, work_id: &WorkId) -> Result<Vec<WorkArtifact>>;
    fn complete_assignment(
        &self,
        assignment: WorkAssignment,
        turn: WorkTurn,
        artifacts: Vec<WorkArtifact>,
    ) -> Result<()>;
    fn finish_assignment(
        &self,
        assignment: WorkAssignment,
        result: ParticipantResult,
        turn: WorkTurn,
        artifacts: Vec<WorkArtifact>,
    ) -> Result<()>;
    fn context(&self, work_id: &WorkId) -> Result<WorkContext>;
    fn find_participant(&self, work_id: &WorkId, name: &str) -> Result<Option<Participant>> {
        Ok(self
            .participants(work_id)?
            .into_iter()
            .find(|participant| participant.name == name))
    }
    fn load_turn(&self, _id: &TurnId) -> Result<Option<WorkTurn>> {
        Ok(None)
    }
    fn load_participant(&self, _id: &ParticipantId) -> Result<Option<Participant>> {
        Ok(None)
    }
    fn load_artifact(&self, _id: &ArtifactId) -> Result<Option<WorkArtifact>> {
        Ok(None)
    }
    fn load_result(&self, _id: &str) -> Result<Option<ParticipantResult>> {
        Ok(None)
    }
    fn conversations(&self, work_id: &WorkId) -> Result<Vec<WorkConversation>>;
    fn import_contribution(&self, conversation: WorkConversation, turn: WorkTurn) -> Result<()>;
    fn load_conversation(&self, _id: &ConversationId) -> Result<Option<WorkConversation>> {
        Ok(None)
    }
    fn create_proposal(&self, proposal: WorkProposal) -> Result<()>;
    fn proposals(&self, work_id: &WorkId) -> Result<Vec<WorkProposal>>;
    fn load_proposal(&self, id: &ProposalId) -> Result<Option<WorkProposal>>;
    fn set_proposal_status(&self, id: &ProposalId, status: ProposalStatus) -> Result<WorkProposal>;
    fn review_proposal(
        &self,
        review: ProposalReview,
    ) -> Result<(WorkProposal, Option<WorkDecision>)>;
    fn reviews(&self, proposal_id: &ProposalId) -> Result<Vec<ProposalReview>>;
    fn load_review(&self, id: &ReviewId) -> Result<Option<ProposalReview>>;
}
