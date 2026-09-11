use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ArtifactKind, ContextGraph, ContextGraphStore, ContextNodeKind, GraphQueryOptions, Result,
    StateError, Timestamp, WorkId, WorkStore,
};

pub const AI_CONTEXT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiParticipant {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiMessage {
    pub id: String,
    pub participant_id: String,
    pub role: String,
    pub content: String,
    pub created_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AiMessageSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiMessageSource {
    pub source: String,
    pub import_id: String,
    pub source_conversation_id: String,
    pub source_message_id: String,
    pub source_timestamp: Option<Timestamp>,
    pub imported_at: Timestamp,
    pub parent_message_id: Option<String>,
    pub child_message_ids: Vec<String>,
    pub attachment_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiDecision {
    pub id: String,
    pub statement: String,
    pub rationale: Option<String>,
    pub revision: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiOpenQuestion {
    pub id: String,
    pub question: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiArtifact {
    pub id: String,
    pub label: String,
    pub reference: String,
    pub kind: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiHistoryExcerpt {
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub excerpt: String,
    pub relevance: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiWorkStateEntry {
    pub category: String,
    pub id: String,
    pub state: String,
    pub detail: String,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProvenance {
    pub source: String,
    pub source_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub context_fingerprint: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiGraphReference {
    pub node_id: String,
    pub kind: ContextNodeKind,
    pub canonical_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConversationContext {
    pub contract: String,
    pub version: u32,
    pub conversation_id: String,
    pub work_id: Option<WorkId>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub participants: Vec<AiParticipant>,
    pub summary: String,
    pub decisions: Vec<AiDecision>,
    pub open_questions: Vec<AiOpenQuestion>,
    pub artifacts: Vec<AiArtifact>,
    pub recent_messages: Vec<AiMessage>,
    pub relevant_history: Vec<AiHistoryExcerpt>,
    pub work_state: Vec<AiWorkStateEntry>,
    pub provenance: Vec<AiProvenance>,
    #[serde(default)]
    pub graph_context: Vec<AiGraphReference>,
    pub context_fingerprint: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl AiConversationContext {
    pub fn canonical_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(Into::into)
    }

    fn fingerprint(&self) -> Result<String> {
        let mut value = self.clone();
        value.context_fingerprint.clear();
        let serialized = serde_json::to_vec(&value)?;
        Ok(format!("{:x}", Sha256::digest(serialized)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum AiContextEventData {
    #[serde(rename = "conversation.created")]
    ConversationCreated {
        participants: Vec<AiParticipant>,
        summary: String,
    },
    #[serde(rename = "conversation.message.created")]
    MessageCreated { message: AiMessage },
    #[serde(rename = "conversation.message.completed")]
    MessageCompleted {
        message_id: String,
        content: String,
        completed_at: Timestamp,
    },
    #[serde(rename = "conversation.context.updated")]
    ContextUpdated {
        summary: String,
        open_questions: Vec<AiOpenQuestion>,
        relevant_history: Vec<AiHistoryExcerpt>,
    },
    #[serde(rename = "conversation.decision.created")]
    DecisionCreated { decision: AiDecision },
    #[serde(rename = "conversation.decision.revised")]
    DecisionRevised {
        decision_id: String,
        statement: String,
        rationale: Option<String>,
        revision: u32,
    },
    #[serde(rename = "conversation.artifact.attached")]
    ArtifactAttached { artifact: AiArtifact },
    #[serde(rename = "conversation.work.linked")]
    WorkLinked { work_id: WorkId },
    #[serde(rename = "conversation.work.unlinked")]
    WorkUnlinked { work_id: WorkId },
    #[serde(rename = "conversation.continuity.exported")]
    ContinuityExported { context_fingerprint: String },
    #[serde(rename = "conversation.continuity.imported")]
    ContinuityImported {
        context_fingerprint: String,
        source: String,
        #[serde(default)]
        import_id: Option<String>,
        #[serde(default)]
        source_conversation_id: Option<String>,
        #[serde(default)]
        source_created_at: Option<Timestamp>,
        #[serde(default)]
        source_updated_at: Option<Timestamp>,
        #[serde(default)]
        source_metadata: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiContextEvent {
    pub id: String,
    pub conversation_id: String,
    pub occurred_at: Timestamp,
    pub event: AiContextEventData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderExchange {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub service: String,
    pub model: Option<String>,
    pub conversation_id: String,
    #[serde(default)]
    pub work_id: Option<WorkId>,
    #[serde(default)]
    pub recipient_id: Option<crate::RecipientId>,
    #[serde(default)]
    pub provider_profile_id: Option<crate::ProviderProfileId>,
    #[serde(default)]
    pub execution_mode: Option<crate::ExecutionMode>,
    pub context_fingerprint: String,
    #[serde(default)]
    pub context_items: Vec<String>,
    #[serde(default)]
    pub context_bytes: usize,
    pub input_message_id: String,
    pub output_message_id: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub usage: Option<AiProviderUsage>,
    #[serde(default)]
    pub status: AiExchangeStatus,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiExchangeStatus {
    #[default]
    Completed,
    ManualTransferRequired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiResponseStatus {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiResponseAttempt {
    pub id: String,
    pub provider: String,
    pub service: String,
    pub model: Option<String>,
    pub conversation_id: String,
    pub work_id: Option<WorkId>,
    pub input_message_id: String,
    pub output_message_id: Option<String>,
    pub context_fingerprint: String,
    pub request_id: Option<String>,
    pub status: AiResponseStatus,
    pub partial_output: String,
    pub error: Option<String>,
    pub usage: Option<AiProviderUsage>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub trait AiContinuityStore {
    fn append_ai_event(&self, event: AiContextEvent) -> Result<()>;
    fn ai_events(&self, conversation_id: &str) -> Result<Vec<AiContextEvent>>;
    fn ai_conversation_ids(&self) -> Result<Vec<String>>;
    fn record_ai_exchange(&self, exchange: AiProviderExchange) -> Result<()>;
    fn ai_exchanges(&self, conversation_id: &str) -> Result<Vec<AiProviderExchange>>;
    fn record_ai_attempt(&self, attempt: AiResponseAttempt) -> Result<()>;
    fn update_ai_attempt(&self, attempt: AiResponseAttempt) -> Result<()>;
    fn ai_attempts(&self, conversation_id: &str) -> Result<Vec<AiResponseAttempt>>;
    fn relevant_ai_history(
        &self,
        conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<AiHistoryExcerpt>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveConversationOptions {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub recent_message_limit: usize,
    pub decision_limit: usize,
    pub artifact_limit: usize,
    pub history_limit: usize,
    pub provenance_limit: usize,
}

impl Default for ResolveConversationOptions {
    fn default() -> Self {
        Self {
            provider: None,
            model: None,
            recent_message_limit: 24,
            decision_limit: 20,
            artifact_limit: 20,
            history_limit: 12,
            provenance_limit: 20,
        }
    }
}

pub fn resolve_conversation_context<S>(
    store: &S,
    conversation_id: &str,
    options: ResolveConversationOptions,
) -> Result<AiConversationContext>
where
    S: AiContinuityStore + WorkStore,
{
    let mut events = store.ai_events(conversation_id)?;
    events.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let first = events.first().ok_or_else(|| StateError::NotFound {
        entity_type: "ai-conversation".into(),
        id: conversation_id.into(),
    })?;
    let mut participants = BTreeMap::new();
    let mut messages = BTreeMap::new();
    let mut decisions = BTreeMap::new();
    let mut artifacts = BTreeMap::new();
    let mut summary = String::new();
    let mut open_questions = Vec::new();
    let mut relevant_history = Vec::new();
    let mut work_id = None;
    let mut provenance = Vec::new();
    for event in &events {
        match &event.event {
            AiContextEventData::ConversationCreated {
                participants: values,
                summary: value,
            } => {
                for participant in values {
                    participants.insert(participant.id.clone(), participant.clone());
                }
                summary.clone_from(value);
            }
            AiContextEventData::MessageCreated { message } => {
                messages.insert(message.id.clone(), message.clone());
            }
            AiContextEventData::MessageCompleted {
                message_id,
                content,
                completed_at,
            } => {
                let message = messages.get_mut(message_id).ok_or_else(|| {
                    StateError::InvalidEntity(format!(
                        "message completion references unknown message {message_id}"
                    ))
                })?;
                message.content.clone_from(content);
                message.completed_at = Some(*completed_at);
            }
            AiContextEventData::ContextUpdated {
                summary: value,
                open_questions: questions,
                relevant_history: history,
            } => {
                summary.clone_from(value);
                open_questions.clone_from(questions);
                relevant_history.clone_from(history);
            }
            AiContextEventData::DecisionCreated { decision } => {
                decisions.insert(decision.id.clone(), decision.clone());
            }
            AiContextEventData::DecisionRevised {
                decision_id,
                statement,
                rationale,
                revision,
            } => {
                let decision = decisions.get_mut(decision_id).ok_or_else(|| {
                    StateError::InvalidEntity(format!(
                        "decision revision references unknown decision {decision_id}"
                    ))
                })?;
                decision.statement.clone_from(statement);
                decision.rationale.clone_from(rationale);
                decision.revision = *revision;
                decision.updated_at = event.occurred_at;
            }
            AiContextEventData::ArtifactAttached { artifact } => {
                artifacts.insert(artifact.id.clone(), artifact.clone());
            }
            AiContextEventData::WorkLinked { work_id: linked } => work_id = Some(linked.clone()),
            AiContextEventData::WorkUnlinked { work_id: unlinked } => {
                if work_id.as_ref() == Some(unlinked) {
                    work_id = None;
                }
            }
            AiContextEventData::ContinuityExported {
                context_fingerprint,
            } => provenance.push(AiProvenance {
                source: "continuity_export".into(),
                source_id: event.id.clone(),
                provider: None,
                model: None,
                context_fingerprint: Some(context_fingerprint.clone()),
                created_at: event.occurred_at,
            }),
            AiContextEventData::ContinuityImported {
                context_fingerprint,
                source,
                ..
            } => provenance.push(AiProvenance {
                source: source.clone(),
                source_id: event.id.clone(),
                provider: None,
                model: None,
                context_fingerprint: Some(context_fingerprint.clone()),
                created_at: event.occurred_at,
            }),
        }
    }
    let mut work_state = Vec::new();
    if let Some(linked_work_id) = work_id.as_ref() {
        let work = store
            .load_work(linked_work_id)?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "work".into(),
                id: linked_work_id.0.clone(),
            })?;
        work_state.push(AiWorkStateEntry {
            category: "work".into(),
            id: work.id.0.clone(),
            state: format!("{:?}", work.status).to_lowercase(),
            detail: work.objective.unwrap_or(work.title),
            updated_at: work.updated_at,
        });
        for participant in store.participants(linked_work_id)? {
            participants
                .entry(participant.id.0.clone())
                .or_insert(AiParticipant {
                    id: participant.id.0,
                    name: participant.name,
                    role: format!("{:?}", participant.kind).to_lowercase(),
                });
        }
        for decision in store.decisions(linked_work_id)? {
            decisions
                .entry(decision.id.0.clone())
                .or_insert(AiDecision {
                    id: decision.id.0,
                    statement: decision.statement,
                    rationale: decision.rationale,
                    revision: 1,
                    created_at: decision.created_at,
                    updated_at: decision.created_at,
                });
        }
        for artifact in store.artifacts(linked_work_id)? {
            let reference = artifact.path.clone().unwrap_or_default();
            let label = artifact
                .description
                .clone()
                .or(artifact.path.clone())
                .unwrap_or_else(|| artifact.id.0.clone());
            artifacts
                .entry(artifact.id.0.clone())
                .or_insert(AiArtifact {
                    id: artifact.id.0,
                    label,
                    reference,
                    kind: artifact_kind(&artifact.kind),
                    created_at: artifact.created_at,
                });
        }
        for proposal in store.proposals(linked_work_id)? {
            work_state.push(AiWorkStateEntry {
                category: "proposal".into(),
                id: proposal.id.0,
                state: format!("{:?}", proposal.status).to_lowercase(),
                detail: proposal.title,
                updated_at: proposal.created_at,
            });
        }
        for assignment in store.assignments(linked_work_id)? {
            work_state.push(AiWorkStateEntry {
                category: "assignment".into(),
                id: assignment.id.0,
                state: format!("{:?}", assignment.status).to_lowercase(),
                detail: assignment.instruction,
                updated_at: assignment.created_at,
            });
        }
        for execution in store.executions(linked_work_id)? {
            work_state.push(AiWorkStateEntry {
                category: "execution".into(),
                id: execution.id.0,
                state: format!("{:?}", execution.status).to_lowercase(),
                detail: execution.provider,
                updated_at: execution.updated_at,
            });
        }
    }
    for exchange in store.ai_exchanges(conversation_id)? {
        provenance.push(AiProvenance {
            source: "provider_exchange".into(),
            source_id: exchange.id,
            provider: Some(exchange.provider),
            model: exchange.model,
            context_fingerprint: Some(exchange.context_fingerprint),
            created_at: exchange.created_at,
        });
    }
    let history_query = format!(
        "{} {}",
        summary,
        messages
            .values()
            .rev()
            .take(4)
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    relevant_history.extend(store.relevant_ai_history(
        conversation_id,
        &history_query,
        options.history_limit,
    )?);
    let mut participants = participants.into_values().collect::<Vec<_>>();
    participants.sort_by(|left, right| left.id.cmp(&right.id));
    let mut messages = messages.into_values().collect::<Vec<_>>();
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    take_recent(&mut messages, options.recent_message_limit);
    let mut decisions = decisions.into_values().collect::<Vec<_>>();
    decisions.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    take_recent(&mut decisions, options.decision_limit);
    let mut artifacts = artifacts.into_values().collect::<Vec<_>>();
    artifacts.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    take_recent(&mut artifacts, options.artifact_limit);
    relevant_history.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.conversation_id.cmp(&right.conversation_id))
    });
    take_recent(&mut relevant_history, options.history_limit);
    work_state.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.updated_at.cmp(&right.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    provenance.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    take_recent(&mut provenance, options.provenance_limit);
    let mut seen_questions = BTreeSet::new();
    open_questions.retain(|question| seen_questions.insert(question.id.clone()));
    open_questions.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let created_at = first.occurred_at;
    let updated_at = events
        .last()
        .map(|event| event.occurred_at)
        .unwrap_or(created_at);
    let mut context = AiConversationContext {
        contract: "COMBE_AI_CONTEXT".into(),
        version: AI_CONTEXT_VERSION,
        conversation_id: conversation_id.into(),
        work_id,
        provider: options.provider,
        model: options.model,
        participants,
        summary,
        decisions,
        open_questions,
        artifacts,
        recent_messages: messages,
        relevant_history,
        work_state,
        provenance,
        graph_context: Vec::new(),
        context_fingerprint: String::new(),
        created_at,
        updated_at,
    };
    context.context_fingerprint = context.fingerprint()?;
    Ok(context)
}

pub fn resolve_conversation_context_with_graph<S>(
    store: &S,
    conversation_id: &str,
    options: ResolveConversationOptions,
) -> Result<AiConversationContext>
where
    S: AiContinuityStore + WorkStore + ContextGraphStore,
{
    let mut context = resolve_conversation_context(store, conversation_id, options)?;
    let Some(graph) = store.load_context_graph()? else {
        return Ok(context);
    };
    let root = context
        .work_id
        .as_ref()
        .map(|work_id| format!("work:{}", work_id.0))
        .unwrap_or_else(|| format!("conversation:{conversation_id}"));
    let scope = context.work_id.clone();
    if let Ok(subgraph) = graph.traverse(
        &root,
        &GraphQueryOptions {
            max_depth: 3,
            max_nodes: 40,
            max_edges: 80,
            max_bytes: 64 * 1024,
            work_scope: scope,
            ..Default::default()
        },
    ) {
        context.graph_context = subgraph
            .nodes
            .into_iter()
            .map(|node| AiGraphReference {
                canonical_id: node
                    .properties
                    .get("work_id")
                    .or_else(|| node.properties.get("conversation_id"))
                    .or_else(|| node.properties.get("decision_id"))
                    .or_else(|| node.properties.get("artifact_id"))
                    .or_else(|| node.properties.get("file_identity"))
                    .or_else(|| node.properties.get("commit_id"))
                    .or_else(|| node.properties.get("exchange_id"))
                    .or_else(|| node.properties.get("context_fingerprint"))
                    .cloned()
                    .unwrap_or_else(|| node.id.clone()),
                node_id: node.id,
                kind: node.kind,
            })
            .collect();
        context
            .graph_context
            .sort_by(|left, right| left.node_id.cmp(&right.node_id));
        context.context_fingerprint = context.fingerprint()?;
    }
    Ok(context)
}

fn take_recent<T>(values: &mut Vec<T>, limit: usize) {
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
}

fn artifact_kind(kind: &ArtifactKind) -> String {
    format!("{kind:?}").to_lowercase()
}

pub fn new_ai_event(conversation_id: &str, event: AiContextEventData) -> AiContextEvent {
    AiContextEvent {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.into(),
        occurred_at: Utc::now(),
        event,
    }
}
