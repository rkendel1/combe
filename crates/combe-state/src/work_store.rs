use crate::{
    ArtifactId, AssignmentId, Participant, ParticipantId, Result, TurnId, Work, WorkArtifact,
    WorkAssignment, WorkContext, WorkDecision, WorkId, WorkStatus, WorkTurn,
};

pub const CONTEXT_TURN_LIMIT: usize = 50;
pub const CONTEXT_DECISION_LIMIT: usize = 100;
pub const CONTEXT_ARTIFACT_LIMIT: usize = 100;

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
}
