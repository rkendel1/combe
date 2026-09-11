use chrono::Utc;
use combe_state::{
    AssignmentId, ContributionKind, ExecutionId, Participant, ProposalId, ReviewOutcome,
    TurnOrigin, WORK_CONTEXT_VERSION, WorkAction, WorkContext, WorkContribution, WorkRequest,
};
use serde::Deserialize;

use crate::participant_adapter::{AdapterError, ContextPackage};

#[derive(Debug, thiserror::Error)]
pub enum ConversationAdapterError {
    #[error("{0}")]
    Adapter(#[from] AdapterError),
    #[error("invalid ChatGPT response: {0}")]
    InvalidResponse(String),
    #[error("{0}")]
    Protocol(#[from] combe_state::StateError),
}

pub struct PreparedConversationContext {
    pub package: ContextPackage,
    pub rendered: String,
}

pub struct ExternalContribution {
    pub request: WorkRequest,
    pub participant: Participant,
    pub source: TurnOrigin,
    pub explicit_kind: ContributionKind,
    pub title: Option<String>,
    pub proposal_id: Option<ProposalId>,
    pub assignment_id: Option<AssignmentId>,
    pub execution_id: Option<ExecutionId>,
    pub input: String,
}

pub trait ConversationParticipantAdapter {
    fn prepare_context(
        &self,
        context: WorkContext,
        participant: &Participant,
        action: WorkAction,
    ) -> Result<PreparedConversationContext, ConversationAdapterError>;

    fn ingest_contribution(
        &self,
        input: ExternalContribution,
    ) -> Result<WorkContribution, ConversationAdapterError>;
}

pub struct ChatGptAdapter;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseEnvelope {
    protocol: String,
    version: u32,
    work_id: String,
    based_on_revision: u64,
    action: WorkAction,
    result: ResponseResult,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseResult {
    kind: ContributionKind,
    summary: String,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    outcome: Option<ReviewOutcome>,
    #[serde(default)]
    proposal_id: Option<ProposalId>,
    #[serde(default)]
    assignment_id: Option<AssignmentId>,
    #[serde(default)]
    execution_id: Option<ExecutionId>,
}

impl ConversationParticipantAdapter for ChatGptAdapter {
    fn prepare_context(
        &self,
        context: WorkContext,
        participant: &Participant,
        action: WorkAction,
    ) -> Result<PreparedConversationContext, ConversationAdapterError> {
        let conversation_linked = context.conversations.iter().any(|conversation| {
            conversation.participant_id == participant.id
                && conversation.conversation.provider == combe_state::ConversationProvider::ChatGpt
        });
        let mut package = ContextPackage::from_context(context, None)?;
        package.request.participant_id = participant.id.clone();
        package.request.requested_action = action;
        package.request.validate(participant)?;
        let request = match action {
            WorkAction::Inspect => "Inspect the current Work and summarize its coordination state.",
            WorkAction::Propose => "Propose a concrete approach for the current Work.",
            WorkAction::Review => {
                "Review the current execution, result, and artifacts and identify required changes."
            }
            _ => {
                return Err(ConversationAdapterError::InvalidResponse(
                    "ChatGPT supports inspect, propose, and review actions".into(),
                ));
            }
        };
        let rendered = format!(
            "{}\nCHATGPT REQUEST\n{request}\nReturn plain text for explicit import, or a COMBE_WORK_CONTEXT version 2 response envelope retaining based_on_revision {}.\n\nChatGPT context prepared.\n{}Direct ChatGPT transport is unavailable.\nCopy the generated context into your ChatGPT conversation.\nAfter ChatGPT responds, use:\n  combe work chatgpt import {} --kind <message|proposal|review> --revision {}\n",
            package.text(),
            package.request.revision,
            if conversation_linked {
                ""
            } else {
                "No ChatGPT conversation is linked to this Work.\n"
            },
            package.request.work_id,
            package.request.revision
        );
        Ok(PreparedConversationContext { package, rendered })
    }

    fn ingest_contribution(
        &self,
        input: ExternalContribution,
    ) -> Result<WorkContribution, ConversationAdapterError> {
        input.request.validate(&input.participant)?;
        if input.input.trim().is_empty() {
            return Err(ConversationAdapterError::InvalidResponse(
                "response is empty".into(),
            ));
        }
        let structured = input.input.trim_start().starts_with('{');
        let envelope = structured
            .then(|| serde_json::from_str::<ResponseEnvelope>(&input.input))
            .transpose()
            .map_err(|error| ConversationAdapterError::InvalidResponse(error.to_string()))?;
        let (kind, content, title, rationale, outcome, proposal_id, assignment_id, execution_id) =
            if let Some(envelope) = envelope {
                if envelope.protocol != "COMBE_WORK_CONTEXT" {
                    return Err(ConversationAdapterError::InvalidResponse(
                        "protocol must be COMBE_WORK_CONTEXT".into(),
                    ));
                }
                if envelope.version != WORK_CONTEXT_VERSION {
                    return Err(ConversationAdapterError::InvalidResponse(format!(
                        "version must be {WORK_CONTEXT_VERSION}"
                    )));
                }
                if envelope.work_id != input.request.work_id.0 {
                    return Err(ConversationAdapterError::InvalidResponse(
                        "work_id does not match the requested Work".into(),
                    ));
                }
                if envelope.based_on_revision != input.request.revision {
                    return Err(ConversationAdapterError::InvalidResponse(
                        "based_on_revision does not match the prepared context".into(),
                    ));
                }
                if envelope.action != input.request.requested_action {
                    return Err(ConversationAdapterError::InvalidResponse(
                        "action does not match the prepared request".into(),
                    ));
                }
                if envelope.result.kind != input.explicit_kind {
                    return Err(ConversationAdapterError::InvalidResponse(
                        "result kind does not match --kind".into(),
                    ));
                }
                let content = match envelope.result.details {
                    Some(details) if !details.trim().is_empty() => {
                        format!("{}\n\n{details}", envelope.result.summary)
                    }
                    _ => envelope.result.summary,
                };
                (
                    envelope.result.kind,
                    content,
                    envelope.result.title.or(input.title),
                    envelope.result.rationale,
                    envelope.result.outcome,
                    envelope.result.proposal_id.or(input.proposal_id),
                    envelope.result.assignment_id.or(input.assignment_id),
                    envelope.result.execution_id.or(input.execution_id),
                )
            } else {
                (
                    input.explicit_kind,
                    input.input,
                    input.title,
                    None,
                    None,
                    input.proposal_id,
                    input.assignment_id,
                    input.execution_id,
                )
            };
        Ok(WorkContribution {
            work_id: input.request.work_id,
            participant_id: input.participant.id,
            context_version: WORK_CONTEXT_VERSION,
            based_on_revision: input.request.revision,
            source: input.source,
            kind,
            content,
            title,
            rationale,
            review_outcome: outcome,
            proposal_id,
            assignment_id,
            execution_id,
            created_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use combe_state::{
        ParticipantCapabilities, ParticipantId, ParticipantKind, Work, WorkId, WorkState,
        WorkStatus,
    };
    use tempfile::TempDir;

    fn fixture() -> (TempDir, WorkContext, Participant) {
        let directory = TempDir::new().unwrap();
        let work = Work {
            id: WorkId("work-chatgpt".into()),
            workspace_id: directory.path().to_string_lossy().into_owned(),
            title: "ChatGPT adapter".into(),
            objective: Some("Keep Work provider-neutral".into()),
            status: WorkStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let participant = Participant {
            id: ParticipantId("chatgpt".into()),
            work_id: work.id.clone(),
            kind: ParticipantKind::Agent,
            name: "ChatGPT".into(),
            capabilities: ParticipantCapabilities {
                can_propose: true,
                can_review: true,
                can_execute: false,
                can_decide: false,
            },
        };
        let context = WorkContext {
            context_version: WORK_CONTEXT_VERSION,
            revision: 184,
            work,
            participants: vec![participant.clone()],
            recent_turns: Vec::new(),
            active_assignments: Vec::new(),
            decisions: Vec::new(),
            artifacts: Vec::new(),
            conversations: Vec::new(),
            proposals: Vec::new(),
            reviews: Vec::new(),
            state: WorkState::default(),
            executions: Vec::new(),
            assignments: Vec::new(),
            results: Vec::new(),
            execution_reviews: Vec::new(),
        };
        (directory, context, participant)
    }

    fn external(input: String, kind: ContributionKind) -> ExternalContribution {
        let (_, context, participant) = fixture();
        ExternalContribution {
            request: WorkRequest {
                work_id: context.work.id,
                participant_id: participant.id.clone(),
                context_version: WORK_CONTEXT_VERSION,
                revision: 184,
                requested_action: match kind {
                    ContributionKind::Proposal => WorkAction::Propose,
                    ContributionKind::Review => WorkAction::Review,
                    _ => WorkAction::Inspect,
                },
            },
            participant,
            source: TurnOrigin::Local,
            explicit_kind: kind,
            title: Some("Explicit title".into()),
            proposal_id: None,
            assignment_id: None,
            execution_id: None,
            input,
        }
    }

    #[test]
    fn prepares_canonical_bounded_protocol_context() {
        let (_directory, context, participant) = fixture();
        let prepared = ChatGptAdapter
            .prepare_context(context, &participant, WorkAction::Review)
            .unwrap();
        assert_eq!(prepared.package.request.revision, 184);
        assert_eq!(
            prepared.package.request.requested_action,
            WorkAction::Review
        );
        assert!(
            prepared
                .rendered
                .starts_with("COMBE_WORK_CONTEXT\nversion: 2")
        );
        assert!(prepared.rendered.contains("CHATGPT REQUEST"));
        assert!(
            prepared
                .rendered
                .contains("Direct ChatGPT transport is unavailable")
        );
    }

    #[test]
    fn imports_plain_and_structured_proposals_without_provider_state() {
        let plain = ChatGptAdapter
            .ingest_contribution(external(
                "Preserve the adapter boundary".into(),
                ContributionKind::Proposal,
            ))
            .unwrap();
        assert_eq!(plain.kind, ContributionKind::Proposal);
        assert_eq!(plain.based_on_revision, 184);
        let structured = r#"{
            "protocol":"COMBE_WORK_CONTEXT",
            "version":2,
            "work_id":"work-chatgpt",
            "based_on_revision":184,
            "action":"propose",
            "result":{"kind":"proposal","summary":"Use the adapter","title":"Adapter"}
        }"#;
        let contribution = ChatGptAdapter
            .ingest_contribution(external(structured.into(), ContributionKind::Proposal))
            .unwrap();
        assert_eq!(contribution.title.as_deref(), Some("Adapter"));
        assert_eq!(contribution.content, "Use the adapter");
    }

    #[test]
    fn rejects_malformed_structured_output() {
        let error = ChatGptAdapter
            .ingest_contribution(external(
                r#"{"protocol":"OTHER"}"#.into(),
                ContributionKind::Message,
            ))
            .unwrap_err();
        assert!(error.to_string().contains("invalid ChatGPT response"));
    }
}
