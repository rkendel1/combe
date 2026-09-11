use std::path::Path;

use chrono::Utc;
use combe_state::{
    AiContextEventData, AiContinuityStore, AiExchangeStatus, AiMessage, AiParticipant,
    AiProviderExchange, AiProviderUsage, AiResponseAttempt, AiResponseStatus, FeltDbWorkStore,
    ProviderRegistry, ProviderResult, RecipientId, ResolveConversationOptions, WorkId,
    new_ai_event, resolve_conversation_context_with_graph,
};

use crate::credential_store::CredentialStore;
use crate::provider_router::{
    ProviderAdapter, ProviderStreamEvent, RoutingError, RoutingService, provider_name,
};

const CONTEXT_LIMIT: usize = 256 * 1024;
const PARTIAL_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationResponseEvent {
    Started,
    Delta(String),
    Completed,
    Failed(String),
}

pub struct ConversationRuntime<'a> {
    store: &'a FeltDbWorkStore,
    router: RoutingService<'a>,
}

impl<'a> ConversationRuntime<'a> {
    pub fn new(
        store: &'a FeltDbWorkStore,
        credentials: &'a dyn CredentialStore,
        adapters: Vec<&'a dyn ProviderAdapter>,
    ) -> Self {
        Self {
            store,
            router: RoutingService::new(store, credentials, adapters),
        }
    }

    pub fn create(
        &self,
        recipient_id: &RecipientId,
        summary: &str,
        work_id: Option<&WorkId>,
    ) -> Result<String, RoutingError> {
        let recipient = self.store.get_recipient(recipient_id)?;
        let conversation_id = uuid::Uuid::new_v4().to_string();
        self.store.append_ai_event(new_ai_event(
            &conversation_id,
            AiContextEventData::ConversationCreated {
                participants: vec![
                    AiParticipant {
                        id: "human".into(),
                        name: "You".into(),
                        role: "human".into(),
                    },
                    AiParticipant {
                        id: recipient.id.0,
                        name: recipient.name,
                        role: "provider".into(),
                    },
                ],
                summary: summary.trim().into(),
            },
        ))?;
        if let Some(work_id) = work_id {
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::WorkLinked {
                    work_id: work_id.clone(),
                },
            ))?;
        }
        Ok(conversation_id)
    }

    pub fn send(
        &self,
        conversation_id: &str,
        recipient_id: &RecipientId,
        content: &str,
        worktree: &Path,
        events: &mut dyn FnMut(ConversationResponseEvent),
    ) -> Result<ProviderResult, RoutingError> {
        if content.trim().is_empty() {
            return Err(
                combe_state::StateError::InvalidEntity("message cannot be empty".into()).into(),
            );
        }
        let message_id = uuid::Uuid::new_v4().to_string();
        let created_at = Utc::now();
        self.store.append_ai_event(new_ai_event(
            conversation_id,
            AiContextEventData::MessageCreated {
                message: AiMessage {
                    id: message_id.clone(),
                    participant_id: "human".into(),
                    role: "user".into(),
                    content: content.trim().into(),
                    created_at,
                    completed_at: None,
                    source: None,
                },
            },
        ))?;
        self.store.append_ai_event(new_ai_event(
            conversation_id,
            AiContextEventData::MessageCompleted {
                message_id: message_id.clone(),
                content: content.trim().into(),
                completed_at: Utc::now(),
            },
        ))?;
        self.attempt(conversation_id, recipient_id, &message_id, worktree, events)
    }

    pub fn retry(
        &self,
        conversation_id: &str,
        recipient_id: &RecipientId,
        input_message_id: &str,
        worktree: &Path,
        events: &mut dyn FnMut(ConversationResponseEvent),
    ) -> Result<ProviderResult, RoutingError> {
        let exists = self.store.ai_events(conversation_id)?.iter().any(|event| {
            matches!(
                &event.event,
                AiContextEventData::MessageCreated { message }
                    if message.id == input_message_id && message.role == "user"
            )
        });
        if !exists {
            return Err(combe_state::StateError::InvalidEntity(
                "retry input message does not exist".into(),
            )
            .into());
        }
        self.attempt(
            conversation_id,
            recipient_id,
            input_message_id,
            worktree,
            events,
        )
    }

    fn attempt(
        &self,
        conversation_id: &str,
        recipient_id: &RecipientId,
        input_message_id: &str,
        worktree: &Path,
        events: &mut dyn FnMut(ConversationResponseEvent),
    ) -> Result<ProviderResult, RoutingError> {
        let recipient = self.store.get_recipient(recipient_id)?;
        let profile = self.store.get_profile(&recipient.provider_profile_id)?;
        if !recipient.enabled || !profile.enabled || !profile.capabilities.text_generation {
            return Err(RoutingError::Unsupported(recipient.name));
        }
        let context = resolve_with_limit(self.store, conversation_id, &profile)?;
        let work_id = context.work_id.clone();
        let provider = provider_name(profile.service).to_string();
        let prompt = format!(
            "COMBE_AI_CONTEXT v1\n{}\nEND_COMBE_AI_CONTEXT",
            context.canonical_json()?
        );
        let now = Utc::now();
        let mut attempt = AiResponseAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.clone(),
            service: provider.clone(),
            model: profile.model.clone(),
            conversation_id: conversation_id.into(),
            work_id: work_id.clone(),
            input_message_id: input_message_id.into(),
            output_message_id: None,
            context_fingerprint: context.context_fingerprint.clone(),
            request_id: None,
            status: AiResponseStatus::Started,
            partial_output: String::new(),
            error: None,
            usage: None,
            created_at: now,
            updated_at: now,
        };
        self.store.record_ai_attempt(attempt.clone())?;
        let credential = self.router.credential(&profile)?;
        let adapter = self.router.adapter(&profile)?;
        let mut partial = String::new();
        let result = adapter.send_stream(
            &profile,
            worktree,
            &prompt,
            credential.as_ref(),
            &mut |event| match event {
                ProviderStreamEvent::Started => events(ConversationResponseEvent::Started),
                ProviderStreamEvent::Delta(delta) => {
                    push_bounded(&mut partial, &delta, PARTIAL_LIMIT);
                    events(ConversationResponseEvent::Delta(delta));
                }
                ProviderStreamEvent::Completed => {}
            },
        );
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let failure = sanitized_failure(&error);
                attempt.status = AiResponseStatus::Failed;
                attempt.partial_output = partial;
                attempt.error = Some(failure.clone());
                attempt.updated_at = Utc::now();
                self.store.update_ai_attempt(attempt)?;
                events(ConversationResponseEvent::Failed(failure));
                return Err(error);
            }
        };
        let output_message_id = uuid::Uuid::new_v4().to_string();
        let output_at = Utc::now();
        self.store.append_ai_event(new_ai_event(
            conversation_id,
            AiContextEventData::MessageCreated {
                message: AiMessage {
                    id: output_message_id.clone(),
                    participant_id: recipient.id.0.clone(),
                    role: "assistant".into(),
                    content: result.content.clone(),
                    created_at: output_at,
                    completed_at: None,
                    source: None,
                },
            },
        ))?;
        self.store.append_ai_event(new_ai_event(
            conversation_id,
            AiContextEventData::MessageCompleted {
                message_id: output_message_id.clone(),
                content: result.content.clone(),
                completed_at: Utc::now(),
            },
        ))?;
        let usage = result.usage.as_ref().map(|value| AiProviderUsage {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        });
        self.store.record_ai_exchange(AiProviderExchange {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.clone(),
            service: provider,
            model: profile.model.clone(),
            conversation_id: conversation_id.into(),
            work_id,
            recipient_id: Some(recipient.id.clone()),
            provider_profile_id: Some(profile.id.clone()),
            execution_mode: Some(profile.execution_mode),
            context_fingerprint: context.context_fingerprint,
            context_items: Vec::new(),
            context_bytes: 0,
            input_message_id: input_message_id.into(),
            output_message_id: output_message_id.clone(),
            request_id: result.provider_request_id.clone(),
            usage: usage.clone(),
            status: AiExchangeStatus::Completed,
            duration_ms: None,
            error: None,
            created_at: Utc::now(),
        })?;
        attempt.status = AiResponseStatus::Completed;
        attempt.output_message_id = Some(output_message_id);
        attempt.request_id.clone_from(&result.provider_request_id);
        attempt.partial_output = result.content.clone();
        attempt.usage = usage;
        attempt.updated_at = Utc::now();
        self.store.update_ai_attempt(attempt)?;
        events(ConversationResponseEvent::Completed);
        Ok(result)
    }
}

fn resolve_with_limit(
    store: &FeltDbWorkStore,
    conversation_id: &str,
    profile: &combe_state::ProviderProfile,
) -> Result<combe_state::AiConversationContext, RoutingError> {
    let mut options = ResolveConversationOptions {
        provider: Some(provider_name(profile.service).into()),
        model: profile.model.clone(),
        ..Default::default()
    };
    loop {
        let context =
            resolve_conversation_context_with_graph(store, conversation_id, options.clone())?;
        if context.canonical_json()?.len() <= CONTEXT_LIMIT {
            return Ok(context);
        }
        if options.history_limit > 0 {
            options.history_limit /= 2;
        } else if options.provenance_limit > 0 {
            options.provenance_limit /= 2;
        } else if options.artifact_limit > 0 {
            options.artifact_limit /= 2;
        } else if options.decision_limit > 4 {
            options.decision_limit /= 2;
        } else if options.recent_message_limit > 4 {
            options.recent_message_limit /= 2;
        } else {
            return Err(RoutingError::Transport(
                "Resolved Combe context exceeds the provider limit".into(),
            ));
        }
    }
}

fn push_bounded(target: &mut String, value: &str, limit: usize) {
    let remaining = limit.saturating_sub(target.len());
    if remaining == 0 {
        return;
    }
    let mut end = remaining.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&value[..end]);
}

fn sanitized_failure(error: &RoutingError) -> String {
    let value = error.to_string().to_lowercase();
    if value.contains("401") || value.contains("authentication") || value.contains("credential") {
        "Provider authentication failed".into()
    } else if value.contains("429") || value.contains("rate limit") {
        "Provider rate limit reached".into()
    } else if value.contains("timeout") || value.contains("timed out") {
        "Provider request timed out".into()
    } else if value.contains("context") && value.contains("limit") {
        "Provider context limit exceeded".into()
    } else {
        "Provider response failed".into()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use combe_state::{
        ChatGptHistoryConversation, ChatGptHistoryMessage, ChatGptHistoryStore,
        ChatGptImportSource, CredentialRef, ExecutionMode, Participant, ParticipantKind,
        ProviderCapabilities, ProviderProfile, ProviderProfileId, ProviderResultStatus,
        ProviderService, ProviderUsage, Recipient, Work, WorkStore,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::credential_store::{Credential, CredentialStore, MemoryCredentialStore};

    struct StreamingAdapter {
        calls: Mutex<Vec<(String, String)>>,
    }

    impl ProviderAdapter for StreamingAdapter {
        fn service(&self) -> ProviderService {
            ProviderService::ChatGpt
        }

        fn supports(&self, profile: &ProviderProfile) -> bool {
            profile.service == ProviderService::ChatGpt
                && profile.execution_mode == ExecutionMode::HttpApi
        }

        fn send(
            &self,
            _profile: &ProviderProfile,
            _worktree: &Path,
            _message: &str,
            _credential: Option<&Credential>,
        ) -> Result<ProviderResult, RoutingError> {
            unreachable!()
        }

        fn send_stream(
            &self,
            profile: &ProviderProfile,
            _worktree: &Path,
            message: &str,
            credential: Option<&Credential>,
            events: &mut dyn FnMut(ProviderStreamEvent),
        ) -> Result<ProviderResult, RoutingError> {
            assert!(credential.is_some());
            let mut calls = self.calls.lock().unwrap();
            calls.push((profile.model.clone().unwrap(), message.into()));
            events(ProviderStreamEvent::Started);
            if calls.len() == 1 {
                events(ProviderStreamEvent::Delta("partial".into()));
                return Err(RoutingError::Transport("request timed out".into()));
            }
            events(ProviderStreamEvent::Delta("final ".into()));
            events(ProviderStreamEvent::Delta("answer".into()));
            events(ProviderStreamEvent::Completed);
            Ok(ProviderResult {
                provider_profile_id: profile.id.clone(),
                model: profile.model.clone(),
                content: "final answer".into(),
                status: ProviderResultStatus::Completed,
                usage: Some(ProviderUsage {
                    input_tokens: Some(12),
                    output_tokens: Some(2),
                }),
                provider_request_id: Some("request-2".into()),
            })
        }

        fn test(
            &self,
            _profile: &ProviderProfile,
            _credential: Option<&Credential>,
        ) -> Result<String, RoutingError> {
            Ok("available".into())
        }
    }

    #[test]
    fn failure_retry_streaming_model_switch_and_work_provenance_are_canonical() {
        let directory = TempDir::new().unwrap();
        let store = FeltDbWorkStore::open(directory.path().join("work.db")).unwrap();
        let work = Work::new(
            directory.path().to_string_lossy().into_owned(),
            "AppPort continuity".into(),
            Some("Recall AppPort service decisions".into()),
        );
        store
            .create_work(
                work.clone(),
                vec![Participant::new(
                    work.id.clone(),
                    ParticipantKind::Human,
                    "Human".into(),
                )],
            )
            .unwrap();
        let credential_ref = CredentialRef("openai/test".into());
        let mut profile = ProviderProfile {
            id: ProviderProfileId::new(),
            name: "ChatGPT".into(),
            service: ProviderService::ChatGpt,
            model: Some("model-a".into()),
            capabilities: ProviderCapabilities {
                text_generation: true,
                code_execution: false,
                structured_output: true,
            },
            execution_mode: ExecutionMode::HttpApi,
            credential_ref: Some(credential_ref.clone()),
            endpoint: None,
            enabled: true,
        };
        let recipient = Recipient {
            id: RecipientId::new(),
            name: "ChatGPT".into(),
            provider_profile_id: profile.id.clone(),
            enabled: true,
        };
        store.create_profile(profile.clone()).unwrap();
        store.create_recipient(recipient.clone()).unwrap();
        let imported_at = Utc::now();
        store
            .import_chatgpt_source(
                ChatGptImportSource {
                    id: "export-1".into(),
                    provenance: "chatgpt_export".into(),
                    display_name: "ChatGPT export".into(),
                    imported_at,
                    conversation_count: 2,
                    fingerprint: "source-fingerprint".into(),
                },
                vec![
                    ChatGptHistoryConversation {
                        source_id: "export-1".into(),
                        conversation_id: "relevant".into(),
                        title: Some("AppPort architecture".into()),
                        created_at: Some(imported_at),
                        updated_at: Some(imported_at),
                        messages: vec![ChatGptHistoryMessage {
                            id: "history-1".into(),
                            parent_id: None,
                            child_ids: Vec::new(),
                            role: Some("assistant".into()),
                            created_at: Some(imported_at),
                            content: "AppPort service decisions keep credentials in Keychain."
                                .into(),
                            attachments: Vec::new(),
                            fingerprint: "message-1".into(),
                        }],
                        source_metadata: Default::default(),
                        fingerprint: "conversation-1".into(),
                    },
                    ChatGptHistoryConversation {
                        source_id: "export-1".into(),
                        conversation_id: "unrelated".into(),
                        title: Some("Garden notes".into()),
                        created_at: Some(imported_at),
                        updated_at: Some(imported_at),
                        messages: vec![ChatGptHistoryMessage {
                            id: "history-2".into(),
                            parent_id: None,
                            child_ids: Vec::new(),
                            role: Some("user".into()),
                            created_at: Some(imported_at),
                            content: "Plant tomatoes beside basil.".into(),
                            attachments: Vec::new(),
                            fingerprint: "message-2".into(),
                        }],
                        source_metadata: Default::default(),
                        fingerprint: "conversation-2".into(),
                    },
                ],
            )
            .unwrap();
        let credentials = MemoryCredentialStore::new();
        credentials
            .set(&credential_ref, Credential::new(b"super-secret-key"))
            .unwrap();
        let adapter = StreamingAdapter {
            calls: Mutex::new(Vec::new()),
        };
        let runtime = ConversationRuntime::new(&store, &credentials, vec![&adapter]);
        let conversation_id = runtime
            .create(&recipient.id, "AppPort service decisions", Some(&work.id))
            .unwrap();
        let mut runtime_events = Vec::new();
        assert!(
            runtime
                .send(
                    &conversation_id,
                    &recipient.id,
                    "What did we decide about AppPort services?",
                    directory.path(),
                    &mut |event| runtime_events.push(event),
                )
                .is_err()
        );
        let context = combe_state::resolve_conversation_context(
            &store,
            &conversation_id,
            ResolveConversationOptions::default(),
        )
        .unwrap();
        assert_eq!(context.work_id, Some(work.id.clone()));
        assert_eq!(context.recent_messages.len(), 1);
        assert_eq!(store.ai_attempts(&conversation_id).unwrap().len(), 1);
        assert_eq!(
            store.ai_attempts(&conversation_id).unwrap()[0].status,
            AiResponseStatus::Failed
        );
        assert_eq!(
            store.ai_attempts(&conversation_id).unwrap()[0].partial_output,
            "partial"
        );
        let input_id = context.recent_messages[0].id.clone();
        profile.model = Some("model-b".into());
        store.update_profile(profile).unwrap();
        runtime
            .retry(
                &conversation_id,
                &recipient.id,
                &input_id,
                directory.path(),
                &mut |event| runtime_events.push(event),
            )
            .unwrap();
        let context = combe_state::resolve_conversation_context(
            &store,
            &conversation_id,
            ResolveConversationOptions::default(),
        )
        .unwrap();
        assert_eq!(context.recent_messages.len(), 2);
        assert_eq!(context.recent_messages[1].content, "final answer");
        let attempts = store.ai_attempts(&conversation_id).unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1].model.as_deref(), Some("model-b"));
        assert_eq!(attempts[1].status, AiResponseStatus::Completed);
        let exchanges = store.ai_exchanges(&conversation_id).unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(
            exchanges[0].context_fingerprint,
            attempts[1].context_fingerprint
        );
        assert_eq!(exchanges[0].work_id, Some(work.id));
        assert_eq!(exchanges[0].request_id.as_deref(), Some("request-2"));
        assert_eq!(adapter.calls.lock().unwrap()[1].0, "model-b");
        assert!(
            adapter.calls.lock().unwrap()[1]
                .1
                .contains("COMBE_AI_CONTEXT v1")
        );
        assert!(
            adapter.calls.lock().unwrap()[0]
                .1
                .contains("AppPort service decisions")
        );
        assert!(
            !adapter.calls.lock().unwrap()[0]
                .1
                .contains("Plant tomatoes")
        );
        let persisted = serde_json::to_string(&(
            store.ai_events(&conversation_id).unwrap(),
            attempts,
            exchanges,
        ))
        .unwrap();
        assert!(!persisted.contains("super-secret-key"));
    }
}
