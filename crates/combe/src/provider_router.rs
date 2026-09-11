use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::Utc;
use combe_state::{
    AiContextEventData, AiContinuityStore, AiExchangeStatus, AiMessage, AiParticipant,
    AiProviderExchange, AiProviderUsage, ContextAssembler, ContextPackage, ContextRequest,
    CredentialRef, FeltDbWorkStore, FeltDbWorkspaceStore, MessageId, ProviderProfile,
    ProviderRegistry, ProviderResult, ProviderResultStatus, ProviderService, ProviderUsage,
    RecipientId, RoutedProviderResult, WorkId, WorkMessage, WorkStore, WorkspaceStore,
    new_ai_event,
};
use serde_json::{Value, json};

use crate::credential_store::{Credential, CredentialError, CredentialStore};

const RESULT_LIMIT: usize = 64 * 1024;
const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const PROVIDER_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error(transparent)]
    State(#[from] combe_state::StateError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("recipient is disabled: {0}")]
    RecipientDisabled(String),
    #[error("provider profile is disabled: {0}")]
    ProfileDisabled(String),
    #[error("recipient does not support text messages: {0}")]
    Unsupported(String),
    #[error("worktree does not exist: {0}")]
    MissingWorktree(String),
    #[error("provider transport is unavailable: {0:?}")]
    MissingTransport(ProviderService),
    #[error("provider request failed: {0}")]
    Transport(String),
}

pub trait ProviderAdapter: Send + Sync {
    fn service(&self) -> ProviderService;
    fn supports(&self, profile: &ProviderProfile) -> bool {
        self.service() == profile.service
    }
    fn send(
        &self,
        profile: &ProviderProfile,
        worktree: &Path,
        message: &str,
        credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError>;
    fn send_request(
        &self,
        request: &ProviderRequest<'_>,
        credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError> {
        let message = request.context.provider_text()?;
        self.send(request.profile, request.worktree, &message, credential)
    }
    fn send_stream(
        &self,
        profile: &ProviderProfile,
        worktree: &Path,
        message: &str,
        credential: Option<&Credential>,
        events: &mut dyn FnMut(ProviderStreamEvent),
    ) -> Result<ProviderResult, RoutingError> {
        events(ProviderStreamEvent::Started);
        let result = self.send(profile, worktree, message, credential)?;
        if !result.content.is_empty() {
            events(ProviderStreamEvent::Delta(result.content.clone()));
        }
        events(ProviderStreamEvent::Completed);
        Ok(result)
    }
    fn test(
        &self,
        profile: &ProviderProfile,
        credential: Option<&Credential>,
    ) -> Result<String, RoutingError>;
}

pub struct ProviderRequest<'a> {
    pub profile: &'a ProviderProfile,
    pub worktree: &'a Path,
    pub context: &'a ContextPackage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStreamEvent {
    Started,
    Delta(String),
    Completed,
}

pub trait MessageRouter {
    fn send(
        &self,
        work_id: &WorkId,
        recipient_id: &RecipientId,
        message: &str,
    ) -> Result<ProviderResult, RoutingError>;
}

pub struct RoutingService<'a> {
    store: &'a FeltDbWorkStore,
    credentials: &'a dyn CredentialStore,
    adapters: Vec<&'a dyn ProviderAdapter>,
}

impl<'a> RoutingService<'a> {
    pub fn new(
        store: &'a FeltDbWorkStore,
        credentials: &'a dyn CredentialStore,
        adapters: Vec<&'a dyn ProviderAdapter>,
    ) -> Self {
        Self {
            store,
            credentials,
            adapters,
        }
    }

    pub fn test_profile(&self, profile: &ProviderProfile) -> Result<String, RoutingError> {
        let credential = self.credential(profile)?;
        self.adapter(profile)?.test(profile, credential.as_ref())
    }

    pub fn preview_context(
        &self,
        work_id: &WorkId,
        recipient_id: &RecipientId,
        message: &str,
    ) -> Result<ContextPackage, RoutingError> {
        Ok(ContextAssembler::assemble(
            self.store,
            &ContextRequest::new(work_id.clone(), recipient_id.clone(), message),
        )?)
    }

    pub(crate) fn adapter(
        &self,
        profile: &ProviderProfile,
    ) -> Result<&dyn ProviderAdapter, RoutingError> {
        self.adapters
            .iter()
            .copied()
            .find(|adapter| adapter.supports(profile))
            .ok_or(RoutingError::MissingTransport(profile.service))
    }

    pub(crate) fn credential(
        &self,
        profile: &ProviderProfile,
    ) -> Result<Option<Credential>, RoutingError> {
        profile
            .credential_ref
            .as_ref()
            .map(|reference| self.credentials.get(reference))
            .transpose()
            .map_err(Into::into)
    }
}

impl MessageRouter for RoutingService<'_> {
    fn send(
        &self,
        work_id: &WorkId,
        recipient_id: &RecipientId,
        message: &str,
    ) -> Result<ProviderResult, RoutingError> {
        if message.trim().is_empty() {
            return Err(
                combe_state::StateError::InvalidEntity("message cannot be empty".into()).into(),
            );
        }
        let work =
            self.store
                .load_work(work_id)?
                .ok_or_else(|| combe_state::StateError::NotFound {
                    entity_type: "work".into(),
                    id: work_id.0.clone(),
                })?;
        let recipient = self.store.get_recipient(recipient_id)?;
        if !recipient.enabled {
            return Err(RoutingError::RecipientDisabled(recipient.name));
        }
        let profile = self.store.get_profile(&recipient.provider_profile_id)?;
        if !profile.enabled {
            return Err(RoutingError::ProfileDisabled(profile.name));
        }
        if !profile.capabilities.text_generation {
            return Err(RoutingError::Unsupported(recipient.name));
        }
        let worktree_path = FeltDbWorkspaceStore::for_combe()
            .ok()
            .and_then(|store| store.load_workspace(&work.workspace_id).ok().flatten())
            .map(|workspace| workspace.path)
            .unwrap_or(work.workspace_id);
        let worktree = Path::new(&worktree_path);
        if !worktree.is_dir() {
            return Err(RoutingError::MissingWorktree(worktree_path));
        }
        let package = self.preview_context(work_id, recipient_id, message)?;
        let sender = self
            .store
            .find_participant(work_id, "Human")?
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("Work has no Human participant".into())
            })?;
        let conversation_id = format!("work:{}", work_id.0);
        let existing_events = self.store.ai_events(&conversation_id)?;
        if existing_events.is_empty() {
            let participants = self
                .store
                .participants(work_id)?
                .into_iter()
                .map(|participant| AiParticipant {
                    id: participant.id.0,
                    name: participant.name,
                    role: format!("{:?}", participant.kind).to_lowercase(),
                })
                .collect();
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::ConversationCreated {
                    participants,
                    summary: work.objective.clone().unwrap_or_else(|| work.title.clone()),
                },
            ))?;
        }
        if !existing_events.iter().any(|event| {
            matches!(
                &event.event,
                AiContextEventData::WorkLinked { work_id: linked } if linked == work_id
            )
        }) {
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::WorkLinked {
                    work_id: work_id.clone(),
                },
            ))?;
        }
        let input_message_id = uuid::Uuid::new_v4().to_string();
        let input_created_at = Utc::now();
        self.store.append_ai_event(new_ai_event(
            &conversation_id,
            AiContextEventData::MessageCreated {
                message: AiMessage {
                    id: input_message_id.clone(),
                    participant_id: sender.id.0.clone(),
                    role: "user".into(),
                    content: message.trim().into(),
                    created_at: input_created_at,
                    completed_at: None,
                    source: None,
                },
            },
        ))?;
        self.store.append_ai_event(new_ai_event(
            &conversation_id,
            AiContextEventData::MessageCompleted {
                message_id: input_message_id.clone(),
                content: message.trim().into(),
                completed_at: Utc::now(),
            },
        ))?;
        let provider = provider_name(profile.service).to_string();
        let message_id = MessageId::new();
        self.store.add_message(WorkMessage {
            id: message_id.clone(),
            work_id: work_id.clone(),
            sender_participant_id: sender.id.clone(),
            recipient_id: recipient.id.clone(),
            provider_profile_id: profile.id.clone(),
            service: profile.service,
            model: profile.model.clone(),
            execution_mode: profile.execution_mode,
            content: message.into(),
            created_at: Utc::now(),
        })?;
        let started = Instant::now();
        let result = self.credential(&profile).and_then(|credential| {
            self.adapter(&profile)?.send_request(
                &ProviderRequest {
                    profile: &profile,
                    worktree,
                    context: &package,
                },
                credential.as_ref(),
            )
        });
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let output_message_id = uuid::Uuid::new_v4().to_string();
                let detail = error.to_string();
                self.store.append_ai_event(new_ai_event(
                    &conversation_id,
                    AiContextEventData::MessageCreated {
                        message: AiMessage {
                            id: output_message_id.clone(),
                            participant_id: recipient.id.0.clone(),
                            role: "assistant".into(),
                            content: detail.clone(),
                            created_at: Utc::now(),
                            completed_at: None,
                            source: None,
                        },
                    },
                ))?;
                self.store.append_ai_event(new_ai_event(
                    &conversation_id,
                    AiContextEventData::MessageCompleted {
                        message_id: output_message_id.clone(),
                        content: detail.clone(),
                        completed_at: Utc::now(),
                    },
                ))?;
                self.store.record_ai_exchange(AiProviderExchange {
                    id: uuid::Uuid::new_v4().to_string(),
                    provider,
                    service: provider_name(profile.service).into(),
                    model: profile.model.clone(),
                    conversation_id,
                    work_id: Some(work_id.clone()),
                    recipient_id: Some(recipient.id.clone()),
                    provider_profile_id: Some(profile.id.clone()),
                    execution_mode: Some(profile.execution_mode),
                    context_fingerprint: package.fingerprint.clone(),
                    context_items: package.items.iter().map(|item| item.id.clone()).collect(),
                    context_bytes: package.bytes,
                    input_message_id,
                    output_message_id,
                    request_id: None,
                    usage: None,
                    status: AiExchangeStatus::Failed,
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                    error: Some(detail.clone()),
                    created_at: Utc::now(),
                })?;
                self.store.add_provider_result(RoutedProviderResult {
                    message_id,
                    work_id: work_id.clone(),
                    recipient_id: recipient.id,
                    provider_profile_id: profile.id.clone(),
                    service: profile.service,
                    model: profile.model.clone(),
                    execution_mode: profile.execution_mode,
                    result: ProviderResult {
                        provider_profile_id: profile.id,
                        model: profile.model,
                        content: detail,
                        status: ProviderResultStatus::Failed,
                        usage: None,
                        provider_request_id: None,
                    },
                    created_at: Utc::now(),
                })?;
                return Err(error);
            }
        };
        if result.status != ProviderResultStatus::ManualTransferRequired {
            let output_message_id = uuid::Uuid::new_v4().to_string();
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::MessageCreated {
                    message: AiMessage {
                        id: output_message_id.clone(),
                        participant_id: recipient.id.0.clone(),
                        role: "assistant".into(),
                        content: result.content.clone(),
                        created_at: Utc::now(),
                        completed_at: None,
                        source: None,
                    },
                },
            ))?;
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::MessageCompleted {
                    message_id: output_message_id.clone(),
                    content: result.content.clone(),
                    completed_at: Utc::now(),
                },
            ))?;
            self.store.record_ai_exchange(AiProviderExchange {
                id: uuid::Uuid::new_v4().to_string(),
                provider,
                service: provider_name(profile.service).into(),
                model: profile.model.clone(),
                conversation_id,
                work_id: Some(work_id.clone()),
                recipient_id: Some(recipient.id.clone()),
                provider_profile_id: Some(profile.id.clone()),
                execution_mode: Some(profile.execution_mode),
                context_fingerprint: package.fingerprint.clone(),
                context_items: package.items.iter().map(|item| item.id.clone()).collect(),
                context_bytes: package.bytes,
                input_message_id,
                output_message_id,
                request_id: result.provider_request_id.clone(),
                usage: result.usage.as_ref().map(|usage| AiProviderUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                }),
                status: AiExchangeStatus::Completed,
                duration_ms: Some(started.elapsed().as_millis() as u64),
                error: None,
                created_at: Utc::now(),
            })?;
        } else {
            let output_message_id = uuid::Uuid::new_v4().to_string();
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::MessageCreated {
                    message: AiMessage {
                        id: output_message_id.clone(),
                        participant_id: recipient.id.0.clone(),
                        role: "assistant".into(),
                        content: result.content.clone(),
                        created_at: Utc::now(),
                        completed_at: None,
                        source: None,
                    },
                },
            ))?;
            self.store.append_ai_event(new_ai_event(
                &conversation_id,
                AiContextEventData::MessageCompleted {
                    message_id: output_message_id.clone(),
                    content: result.content.clone(),
                    completed_at: Utc::now(),
                },
            ))?;
            self.store.record_ai_exchange(AiProviderExchange {
                id: uuid::Uuid::new_v4().to_string(),
                provider,
                service: provider_name(profile.service).into(),
                model: profile.model.clone(),
                conversation_id,
                work_id: Some(work_id.clone()),
                recipient_id: Some(recipient.id.clone()),
                provider_profile_id: Some(profile.id.clone()),
                execution_mode: Some(profile.execution_mode),
                context_fingerprint: package.fingerprint.clone(),
                context_items: package.items.iter().map(|item| item.id.clone()).collect(),
                context_bytes: package.bytes,
                input_message_id: input_message_id.clone(),
                output_message_id,
                request_id: None,
                usage: None,
                status: AiExchangeStatus::ManualTransferRequired,
                duration_ms: Some(started.elapsed().as_millis() as u64),
                error: None,
                created_at: Utc::now(),
            })?;
        }
        self.store.add_provider_result(RoutedProviderResult {
            message_id,
            work_id: work_id.clone(),
            recipient_id: recipient.id,
            provider_profile_id: profile.id,
            service: profile.service,
            model: profile.model,
            execution_mode: profile.execution_mode,
            result: result.clone(),
            created_at: Utc::now(),
        })?;
        Ok(result)
    }
}

pub(crate) fn provider_name(service: ProviderService) -> &'static str {
    match service {
        ProviderService::Ollama => "ollama",
        ProviderService::ClaudeCode => "claude",
        ProviderService::ChatGpt => "chatgpt",
        ProviderService::OpenAiApi => "openai",
    }
}

pub struct OllamaAdapter {
    client: reqwest::blocking::Client,
}

impl OllamaAdapter {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(PROVIDER_REQUEST_TIMEOUT)
                .build()
                .expect("HTTP client configuration is valid"),
        }
    }

    pub fn models(&self, endpoint: &str) -> Result<Vec<String>, RoutingError> {
        let endpoint = local_ollama_endpoint(endpoint)?;
        let body: Value = self
            .client
            .get(format!("{endpoint}/api/tags"))
            .timeout(PROVIDER_DISCOVERY_TIMEOUT)
            .send()
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?
            .json()
            .map_err(transport)?;
        let mut models = body
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect::<Vec<_>>();
        models.sort();
        models.dedup();
        Ok(models)
    }
}

impl ProviderAdapter for OllamaAdapter {
    fn service(&self) -> ProviderService {
        ProviderService::Ollama
    }

    fn send(
        &self,
        profile: &ProviderProfile,
        _worktree: &Path,
        message: &str,
        _credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError> {
        let endpoint = ollama_endpoint(profile)?;
        let model = required_model(profile)?;
        let response = self
            .client
            .post(format!("{endpoint}/api/generate"))
            .json(&json!({ "model": model, "prompt": message, "stream": false }))
            .send()
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body: Value = response.json().map_err(transport)?;
        Ok(ProviderResult {
            provider_profile_id: profile.id.clone(),
            model: profile.model.clone(),
            content: bounded(body.get("response").and_then(Value::as_str).unwrap_or("")),
            status: ProviderResultStatus::Completed,
            usage: Some(ProviderUsage {
                input_tokens: body.get("prompt_eval_count").and_then(Value::as_u64),
                output_tokens: body.get("eval_count").and_then(Value::as_u64),
            }),
            provider_request_id: request_id,
        })
    }

    fn test(
        &self,
        profile: &ProviderProfile,
        _credential: Option<&Credential>,
    ) -> Result<String, RoutingError> {
        let endpoint = ollama_endpoint(profile)?;
        let model = required_model(profile)?;
        let body: Value = self
            .client
            .get(format!("{endpoint}/api/tags"))
            .timeout(PROVIDER_DISCOVERY_TIMEOUT)
            .send()
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?
            .json()
            .map_err(transport)?;
        let available = body
            .get("models")
            .and_then(Value::as_array)
            .is_some_and(|models| {
                models
                    .iter()
                    .any(|entry| entry.get("name").and_then(Value::as_str) == Some(model))
            });
        if !available {
            return Err(RoutingError::Transport(format!(
                "Ollama model is unavailable: {model}"
            )));
        }
        Ok("Connected · model available".into())
    }
}

pub struct OpenAiAdapter {
    client: reqwest::blocking::Client,
}

pub struct ChatGptProvider {
    client: reqwest::blocking::Client,
}

impl ChatGptProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(PROVIDER_REQUEST_TIMEOUT)
                .build()
                .expect("HTTP client configuration is valid"),
        }
    }
}

impl ProviderAdapter for ChatGptProvider {
    fn service(&self) -> ProviderService {
        ProviderService::ChatGpt
    }

    fn supports(&self, profile: &ProviderProfile) -> bool {
        profile.service == ProviderService::ChatGpt
            && profile.execution_mode == combe_state::ExecutionMode::HttpApi
    }

    fn send(
        &self,
        profile: &ProviderProfile,
        worktree: &Path,
        message: &str,
        credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError> {
        self.send_stream(profile, worktree, message, credential, &mut |_| {})
    }

    fn send_stream(
        &self,
        profile: &ProviderProfile,
        _worktree: &Path,
        message: &str,
        credential: Option<&Credential>,
        events: &mut dyn FnMut(ProviderStreamEvent),
    ) -> Result<ProviderResult, RoutingError> {
        use std::io::BufRead;

        let model = required_model(profile)?;
        let key = credential.ok_or_else(|| missing_credential(profile))?;
        let key = std::str::from_utf8(key.expose())
            .map_err(|_| RoutingError::Transport("OpenAI credential is invalid".into()))?;
        events(ProviderStreamEvent::Started);
        let response = self
            .client
            .post("https://api.openai.com/v1/responses")
            .bearer_auth(key)
            .json(&json!({ "model": model, "input": message, "store": false, "stream": true }))
            .send()
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?;
        let header_request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut content = String::new();
        let mut request_id = header_request_id;
        let mut usage = None;
        for line in std::io::BufReader::new(response).lines() {
            let line = line.map_err(|error| RoutingError::Transport(error.to_string()))?;
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }
            let value: Value = serde_json::from_str(data).map_err(|_| {
                RoutingError::Transport("ChatGPT returned invalid stream data".into())
            })?;
            match value.get("type").and_then(Value::as_str) {
                Some("response.output_text.delta") => {
                    if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                        content.push_str(delta);
                        events(ProviderStreamEvent::Delta(delta.into()));
                    }
                }
                Some("response.completed") => {
                    let response = value.get("response").unwrap_or(&value);
                    request_id = request_id.or_else(|| {
                        response
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    });
                    usage = response.get("usage").map(|usage| ProviderUsage {
                        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
                    });
                }
                Some("error" | "response.failed") => {
                    let message = value
                        .pointer("/error/message")
                        .or_else(|| value.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("ChatGPT response stream failed");
                    return Err(RoutingError::Transport(bounded(message)));
                }
                _ => {}
            }
        }
        events(ProviderStreamEvent::Completed);
        Ok(ProviderResult {
            provider_profile_id: profile.id.clone(),
            model: profile.model.clone(),
            content: bounded(&content),
            status: ProviderResultStatus::Completed,
            usage,
            provider_request_id: request_id,
        })
    }

    fn test(
        &self,
        profile: &ProviderProfile,
        credential: Option<&Credential>,
    ) -> Result<String, RoutingError> {
        OpenAiAdapter::new().test(profile, credential)
    }
}

impl OpenAiAdapter {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
        }
    }
}

impl ProviderAdapter for OpenAiAdapter {
    fn service(&self) -> ProviderService {
        ProviderService::OpenAiApi
    }

    fn send(
        &self,
        profile: &ProviderProfile,
        _worktree: &Path,
        message: &str,
        credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError> {
        let model = required_model(profile)?;
        let key = credential.ok_or_else(|| missing_credential(profile))?;
        let key = std::str::from_utf8(key.expose())
            .map_err(|_| RoutingError::Transport("OpenAI credential is invalid".into()))?;
        let response = self
            .client
            .post("https://api.openai.com/v1/responses")
            .bearer_auth(key)
            .json(&json!({ "model": model, "input": message, "store": false }))
            .send()
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body: Value = response.json().map_err(transport)?;
        let content = body
            .get("output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("content").and_then(Value::as_array))
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ProviderResult {
            provider_profile_id: profile.id.clone(),
            model: profile.model.clone(),
            content: bounded(&content),
            status: ProviderResultStatus::Completed,
            usage: body.get("usage").map(|usage| ProviderUsage {
                input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
            }),
            provider_request_id: request_id
                .or_else(|| body.get("id").and_then(Value::as_str).map(str::to_owned)),
        })
    }

    fn test(
        &self,
        profile: &ProviderProfile,
        credential: Option<&Credential>,
    ) -> Result<String, RoutingError> {
        let key = credential.ok_or_else(|| missing_credential(profile))?;
        let key = std::str::from_utf8(key.expose())
            .map_err(|_| RoutingError::Transport("OpenAI credential is invalid".into()))?;
        self.client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(key)
            .send()
            .map_err(transport)?
            .error_for_status()
            .map_err(transport)?;
        Ok("Authentication succeeded".into())
    }
}

pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    pub fn authentication_status() -> Result<String, RoutingError> {
        let executable = which::which("claude")
            .map_err(|_| RoutingError::Transport("Claude Code CLI was not found in PATH".into()))?;
        let output = Command::new(executable)
            .args(["auth", "status"])
            .output()
            .map_err(|error| RoutingError::Transport(error.to_string()))?;
        if output.status.success() {
            return Ok("Claude Code is signed in through its installed CLI account.".into());
        }
        Err(RoutingError::Transport(
            "Claude Code is not signed in. Choose Sign In in Terminal and complete the browser login."
                .into(),
        ))
    }
}

impl ProviderAdapter for ClaudeCodeAdapter {
    fn service(&self) -> ProviderService {
        ProviderService::ClaudeCode
    }

    fn send(
        &self,
        profile: &ProviderProfile,
        worktree: &Path,
        message: &str,
        _credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError> {
        let executable = which::which("claude")
            .map_err(|_| RoutingError::Transport("Claude Code CLI was not found in PATH".into()))?;
        let mut command = Command::new(executable);
        command
            .args(["--print", "--no-session-persistence"])
            .current_dir(worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = profile.model.as_deref() {
            command.args(["--model", model]);
        }
        let mut child = command
            .spawn()
            .map_err(|error| RoutingError::Transport(error.to_string()))?;
        use std::io::Write;
        child
            .stdin
            .take()
            .ok_or_else(|| RoutingError::Transport("Claude stdin is unavailable".into()))?
            .write_all(message.as_bytes())
            .map_err(|error| RoutingError::Transport(error.to_string()))?;
        let output = child
            .wait_with_output()
            .map_err(|error| RoutingError::Transport(error.to_string()))?;
        if !output.status.success() {
            return Err(RoutingError::Transport(bounded(&String::from_utf8_lossy(
                &output.stderr,
            ))));
        }
        Ok(ProviderResult {
            provider_profile_id: profile.id.clone(),
            model: profile.model.clone(),
            content: bounded(&String::from_utf8_lossy(&output.stdout)),
            status: ProviderResultStatus::Completed,
            usage: None,
            provider_request_id: None,
        })
    }

    fn test(
        &self,
        _profile: &ProviderProfile,
        _credential: Option<&Credential>,
    ) -> Result<String, RoutingError> {
        Self::authentication_status()
    }
}

pub struct ChatGptManualAdapter;

impl ProviderAdapter for ChatGptManualAdapter {
    fn service(&self) -> ProviderService {
        ProviderService::ChatGpt
    }

    fn supports(&self, profile: &ProviderProfile) -> bool {
        profile.service == ProviderService::ChatGpt
            && profile.execution_mode == combe_state::ExecutionMode::ExternalManual
    }

    fn send(
        &self,
        profile: &ProviderProfile,
        _worktree: &Path,
        message: &str,
        _credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError> {
        Ok(ProviderResult {
            provider_profile_id: profile.id.clone(),
            model: profile.model.clone(),
            content: bounded(message),
            status: ProviderResultStatus::ManualTransferRequired,
            usage: None,
            provider_request_id: None,
        })
    }

    fn test(
        &self,
        _profile: &ProviderProfile,
        _credential: Option<&Credential>,
    ) -> Result<String, RoutingError> {
        Ok("External conversation · manual transfer required".into())
    }
}

fn ollama_endpoint(profile: &ProviderProfile) -> Result<String, RoutingError> {
    let endpoint = profile
        .endpoint
        .as_deref()
        .ok_or_else(|| RoutingError::Transport("Ollama endpoint is not configured".into()))?
        .trim_end_matches('/');
    local_ollama_endpoint(endpoint)
}

fn local_ollama_endpoint(endpoint: &str) -> Result<String, RoutingError> {
    let endpoint = endpoint.trim_end_matches('/');
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| RoutingError::Transport("Ollama endpoint is invalid".into()))?;
    if url.scheme() != "http" || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return Err(RoutingError::Transport(
            "Ollama endpoint must be local HTTP".into(),
        ));
    }
    Ok(endpoint.into())
}

fn required_model(profile: &ProviderProfile) -> Result<&str, RoutingError> {
    profile
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| RoutingError::Transport("provider model is not configured".into()))
}

fn missing_credential(profile: &ProviderProfile) -> RoutingError {
    RoutingError::Credential(CredentialError::Missing(
        profile
            .credential_ref
            .as_ref()
            .unwrap_or(&CredentialRef("openai/default".into()))
            .0
            .clone(),
    ))
}

fn transport(error: reqwest::Error) -> RoutingError {
    RoutingError::Transport(error.to_string())
}

fn bounded(value: &str) -> String {
    if value.len() <= RESULT_LIMIT {
        return value.into();
    }
    let mut end = RESULT_LIMIT;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use combe_state::{
        ExecutionMode, Participant, ParticipantKind, ProviderCapabilities, ProviderProfileId,
        Recipient, Work,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::credential_store::MemoryCredentialStore;

    struct RecordingAdapter {
        calls: Mutex<Vec<(String, String)>>,
        status: ProviderResultStatus,
    }

    impl ProviderAdapter for RecordingAdapter {
        fn service(&self) -> ProviderService {
            ProviderService::ChatGpt
        }

        fn send(
            &self,
            profile: &ProviderProfile,
            worktree: &Path,
            message: &str,
            _credential: Option<&Credential>,
        ) -> Result<ProviderResult, RoutingError> {
            self.calls
                .lock()
                .unwrap()
                .push((worktree.to_string_lossy().into_owned(), message.into()));
            Ok(ProviderResult {
                provider_profile_id: profile.id.clone(),
                model: None,
                content: message.into(),
                status: self.status,
                usage: None,
                provider_request_id: None,
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
    fn routes_only_to_the_explicit_recipient_and_records_provenance() {
        let directory = TempDir::new().unwrap();
        std::fs::write(
            directory.path().join("README.md"),
            "# Routed project\nA service package for authenticated application APIs.",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("package.json"),
            r#"{"name":"routed-project","dependencies":{"jsonwebtoken":"latest"}}"#,
        )
        .unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(
            directory.path().join("src/auth.ts"),
            "export function authenticate(token: string) { return verify(token); }",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("src/unrelated.ts"),
            "export const color = 'blue';",
        )
        .unwrap();
        std::fs::write(
            directory.path().join(".env"),
            "OPENAI_API_KEY=sk-private-secret",
        )
        .unwrap();
        let store = FeltDbWorkStore::open(directory.path().join("work.db")).unwrap();
        let work = Work::new(
            directory.path().to_string_lossy().into_owned(),
            "Routed work".into(),
            None,
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
        let profile = ProviderProfile {
            id: ProviderProfileId("chatgpt-profile".into()),
            name: "ChatGPT conversation".into(),
            service: ProviderService::ChatGpt,
            model: None,
            capabilities: ProviderCapabilities {
                text_generation: true,
                code_execution: false,
                structured_output: false,
            },
            execution_mode: ExecutionMode::ExternalManual,
            credential_ref: None,
            endpoint: None,
            enabled: true,
        };
        let recipient = Recipient {
            id: RecipientId("reviewer".into()),
            name: "Reviewer".into(),
            provider_profile_id: profile.id.clone(),
            enabled: true,
        };
        store.create_profile(profile.clone()).unwrap();
        store.create_recipient(recipient.clone()).unwrap();
        let adapter = RecordingAdapter {
            calls: Mutex::new(Vec::new()),
            status: ProviderResultStatus::Completed,
        };
        let credentials = MemoryCredentialStore::new();
        let router = RoutingService::new(&store, &credentials, vec![&adapter]);

        let question = "What does this repository do and how is authentication implemented?";
        let first = router
            .preview_context(&work.id, &recipient.id, question)
            .unwrap();
        let second = router
            .preview_context(&work.id, &recipient.id, question)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.items[0].kind, combe_state::ContextNodeKind::Work);
        assert!(
            first
                .items
                .iter()
                .any(|item| item.kind == combe_state::ContextNodeKind::Worktree)
        );
        assert!(first.bytes <= combe_state::DEFAULT_CONTEXT_BYTES);
        assert!(first.items.iter().any(|item| {
            item.reference == "README.md"
                && item
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("service package"))
        }));
        assert!(first.items.iter().any(|item| {
            item.reference == "package.json"
                && item
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("jsonwebtoken"))
        }));
        assert!(first.items.iter().any(|item| {
            item.reference == "src/auth.ts"
                && item
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("authenticate"))
        }));
        assert!(
            !first
                .items
                .iter()
                .any(|item| item.reference.contains(".env"))
        );
        assert!(
            !first
                .items
                .iter()
                .any(|item| item.reference == "src/unrelated.ts")
        );
        std::fs::write(
            directory.path().join("README.md"),
            "# Routed project\nA changed authenticated API service.",
        )
        .unwrap();
        let changed = router
            .preview_context(&work.id, &recipient.id, question)
            .unwrap();
        assert_ne!(first.fingerprint, changed.fingerprint);

        store
            .set_context_file_hint(combe_state::WorkContextFileHint {
                work_id: work.id.clone(),
                path: "src/unrelated.ts".into(),
                disposition: combe_state::ContextFileDisposition::Include,
                updated_at: Utc::now(),
            })
            .unwrap();
        store
            .set_context_file_hint(combe_state::WorkContextFileHint {
                work_id: work.id.clone(),
                path: "src/auth.ts".into(),
                disposition: combe_state::ContextFileDisposition::Exclude,
                updated_at: Utc::now(),
            })
            .unwrap();
        let hinted = router
            .preview_context(&work.id, &recipient.id, question)
            .unwrap();
        assert!(
            hinted
                .items
                .iter()
                .any(|item| item.reference == "src/unrelated.ts")
        );
        assert!(
            !hinted
                .items
                .iter()
                .any(|item| item.reference == "src/auth.ts")
        );
        store
            .set_context_file_hint(combe_state::WorkContextFileHint {
                work_id: work.id.clone(),
                path: "src/auth.ts".into(),
                disposition: combe_state::ContextFileDisposition::Include,
                updated_at: Utc::now(),
            })
            .unwrap();
        store
            .set_context_file_hint(combe_state::WorkContextFileHint {
                work_id: work.id.clone(),
                path: "src/unrelated.ts".into(),
                disposition: combe_state::ContextFileDisposition::Exclude,
                updated_at: Utc::now(),
            })
            .unwrap();

        router.send(&work.id, &recipient.id, question).unwrap();

        assert_eq!(adapter.calls.lock().unwrap().len(), 1);
        let routed = adapter.calls.lock().unwrap()[0].1.clone();
        assert!(routed.contains("COMBE_CONTEXT_PACKAGE v1"));
        assert!(routed.contains("\"contract\":\"COMBE_CONTEXT_PACKAGE\""));
        assert!(routed.contains("\"display\":\"Routed work\""));
        assert!(routed.contains("\"fingerprint\":"));
        assert!(routed.contains(question));
        assert!(routed.contains("A changed authenticated API service"));
        assert!(routed.contains("export function authenticate"));
        let messages = store.messages(&work.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].recipient_id, recipient.id);
        assert_eq!(messages[0].provider_profile_id, profile.id);
        assert_eq!(messages[0].service, ProviderService::ChatGpt);
        let results = store.provider_results(&work.id).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, messages[0].id);
        assert_eq!(results[0].recipient_id, recipient.id);
        let exchanges = store.ai_exchanges(&format!("work:{}", work.id.0)).unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].provider, "chatgpt");
        assert!(!exchanges[0].context_fingerprint.is_empty());
        assert!(!exchanges[0].context_items.is_empty());
        assert_eq!(exchanges[0].recipient_id.as_ref(), Some(&recipient.id));
        assert!(!exchanges[0].input_message_id.is_empty());
        assert!(!exchanges[0].output_message_id.is_empty());
    }

    #[test]
    fn never_falls_back_when_the_selected_service_has_no_adapter() {
        let directory = TempDir::new().unwrap();
        let store = FeltDbWorkStore::open(directory.path().join("work.db")).unwrap();
        let work = Work::new(
            directory.path().to_string_lossy().into_owned(),
            "No fallback".into(),
            None,
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
        let profile = ProviderProfile {
            id: ProviderProfileId("ollama-profile".into()),
            name: "Local".into(),
            service: ProviderService::Ollama,
            model: Some("local-model".into()),
            capabilities: ProviderCapabilities {
                text_generation: true,
                code_execution: false,
                structured_output: false,
            },
            execution_mode: ExecutionMode::LocalHttp,
            credential_ref: None,
            endpoint: Some("http://127.0.0.1:11434".into()),
            enabled: true,
        };
        let recipient = Recipient {
            id: RecipientId("local".into()),
            name: "Local".into(),
            provider_profile_id: profile.id.clone(),
            enabled: true,
        };
        store.create_profile(profile).unwrap();
        store.create_recipient(recipient.clone()).unwrap();
        let adapter = RecordingAdapter {
            calls: Mutex::new(Vec::new()),
            status: ProviderResultStatus::Completed,
        };
        let credentials = MemoryCredentialStore::new();
        let router = RoutingService::new(&store, &credentials, vec![&adapter]);

        assert!(matches!(
            router.send(&work.id, &recipient.id, "Run locally"),
            Err(RoutingError::MissingTransport(ProviderService::Ollama))
        ));
        assert!(adapter.calls.lock().unwrap().is_empty());
        assert_eq!(store.messages(&work.id).unwrap().len(), 1);
        let results = store.provider_results(&work.id).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].result.status, ProviderResultStatus::Failed);
        let exchanges = store.ai_exchanges(&format!("work:{}", work.id.0)).unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].status, AiExchangeStatus::Failed);
    }
}
