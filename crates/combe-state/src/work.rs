use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type Timestamp = DateTime<Utc>;

macro_rules! id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

id!(WorkId);
id!(ParticipantId);
id!(TurnId);
id!(AssignmentId);
id!(DecisionId);
id!(ArtifactId);
id!(ConversationId);
id!(ProposalId);
id!(ReviewId);

pub type WorkspaceId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Active,
    Paused,
    Completed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub id: WorkId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub objective: Option<String>,
    pub status: WorkStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Work {
    pub fn new(workspace_id: WorkspaceId, title: String, objective: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: WorkId::new(),
            workspace_id,
            title,
            objective,
            status: WorkStatus::Active,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantKind {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    pub id: ParticipantId,
    pub work_id: WorkId,
    pub kind: ParticipantKind,
    pub name: String,
}

impl Participant {
    pub fn new(work_id: WorkId, kind: ParticipantKind, name: String) -> Self {
        Self {
            id: ParticipantId::new(),
            work_id,
            kind,
            name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    Message,
    Analysis,
    Review,
    Implementation,
    Decision,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationProvider {
    ChatGpt,
    Claude,
    Codex,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRef {
    pub id: String,
    pub provider: ConversationProvider,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    #[default]
    Local,
    ExternalConversation(ConversationRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkTurn {
    pub id: TurnId,
    pub work_id: WorkId,
    pub participant_id: ParticipantId,
    pub kind: TurnKind,
    pub content: String,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_id: Option<AssignmentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(default)]
    pub origin: TurnOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkConversation {
    pub id: ConversationId,
    pub work_id: WorkId,
    pub conversation: ConversationRef,
    pub participant_id: ParticipantId,
    pub label: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    Proposed,
    Approved,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkProposal {
    pub id: ProposalId,
    pub work_id: WorkId,
    pub proposed_by: ParticipantId,
    pub title: String,
    pub statement: String,
    pub rationale: Option<String>,
    pub status: ProposalStatus,
    #[serde(default)]
    pub origin: TurnOrigin,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    Approve,
    Reject,
    RequestChanges,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalReview {
    pub id: ReviewId,
    pub proposal_id: ProposalId,
    pub reviewed_by: ParticipantId,
    pub outcome: ReviewOutcome,
    pub comment: Option<String>,
    #[serde(default)]
    pub origin: TurnOrigin,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantResult {
    pub id: String,
    pub work_id: WorkId,
    pub participant_id: ParticipantId,
    pub assignment_id: AssignmentId,
    pub execution_id: String,
    pub exit_status: Option<i32>,
    pub summary: Option<String>,
    pub output: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Pending,
    Active,
    Completed,
    Rejected,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAssignment {
    pub id: AssignmentId,
    pub work_id: WorkId,
    pub from_participant_id: ParticipantId,
    pub to_participant_id: ParticipantId,
    pub instruction: String,
    pub status: AssignmentStatus,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<ProposalId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkDecision {
    pub id: DecisionId,
    pub work_id: WorkId,
    pub statement: String,
    pub rationale: Option<String>,
    pub decided_by: ParticipantId,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<ProposalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<ReviewId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    File,
    Patch,
    Commit,
    Report,
    Review,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkArtifact {
    pub id: ArtifactId,
    pub work_id: WorkId,
    pub kind: ArtifactKind,
    pub path: Option<String>,
    pub description: Option<String>,
    pub created_by: ParticipantId,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkContext {
    pub work: Work,
    pub participants: Vec<Participant>,
    pub recent_turns: Vec<WorkTurn>,
    pub active_assignments: Vec<WorkAssignment>,
    pub decisions: Vec<WorkDecision>,
    pub artifacts: Vec<WorkArtifact>,
    pub conversations: Vec<WorkConversation>,
    #[serde(default)]
    pub proposals: Vec<WorkProposal>,
    #[serde(default)]
    pub reviews: Vec<ProposalReview>,
    #[serde(default)]
    pub state: WorkState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkState {
    pub active_proposals: Vec<WorkProposal>,
    pub approved_proposals: Vec<WorkProposal>,
    pub active_assignments: Vec<WorkAssignment>,
    pub recent_results: Vec<WorkTurn>,
}

#[cfg(test)]
mod tests {
    use super::ConversationProvider;

    #[test]
    fn conversation_providers_round_trip_through_json() {
        for provider in [
            ConversationProvider::ChatGpt,
            ConversationProvider::Claude,
            ConversationProvider::Codex,
            ConversationProvider::Other("local-model".into()),
        ] {
            let json = serde_json::to_string(&provider).unwrap();
            assert_eq!(
                serde_json::from_str::<ConversationProvider>(&json).unwrap(),
                provider
            );
        }
    }
}
