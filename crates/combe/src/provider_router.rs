use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{fmt::Write as _, fs};

use chrono::Utc;
use combe_state::{
    CredentialRef, FeltDbWorkStore, FeltDbWorkspaceStore, MessageId, ProviderProfile,
    ProviderRegistry, ProviderResult, ProviderResultStatus, ProviderService, ProviderUsage,
    RecipientId, RoutedProviderResult, WorkId, WorkMessage, WorkStore, WorkspaceStore,
};
use serde_json::{Value, json};

use crate::credential_store::{Credential, CredentialError, CredentialStore};
use crate::participant_adapter::ContextPackage;

const RESULT_LIMIT: usize = 64 * 1024;
const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const PROVIDER_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const REPOSITORY_SNAPSHOT_LIMIT: usize = 32 * 1024;

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
    #[error("Work context could not be prepared: {0}")]
    Context(String),
}

pub trait ProviderAdapter: Send + Sync {
    fn service(&self) -> ProviderService;
    fn send(
        &self,
        profile: &ProviderProfile,
        worktree: &Path,
        message: &str,
        credential: Option<&Credential>,
    ) -> Result<ProviderResult, RoutingError>;
    fn test(
        &self,
        profile: &ProviderProfile,
        credential: Option<&Credential>,
    ) -> Result<String, RoutingError>;
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
        self.adapter(profile.service)?
            .test(profile, credential.as_ref())
    }

    fn adapter(&self, service: ProviderService) -> Result<&dyn ProviderAdapter, RoutingError> {
        self.adapters
            .iter()
            .copied()
            .find(|adapter| adapter.service() == service)
            .ok_or(RoutingError::MissingTransport(service))
    }

    fn credential(&self, profile: &ProviderProfile) -> Result<Option<Credential>, RoutingError> {
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
        let credential = self.credential(&profile)?;
        let context = self.store.context(work_id)?;
        let package = ContextPackage::from_context(context, None)
            .map_err(|error| RoutingError::Context(error.to_string()))?;
        let routed_message = format!(
            "{}\n{}\nROUTED REQUEST\n{}\nEND_ROUTED_REQUEST\n",
            package.text(),
            repository_snapshot(worktree),
            message.trim()
        );
        let result = self.adapter(profile.service)?.send(
            &profile,
            worktree,
            &routed_message,
            credential.as_ref(),
        )?;
        let sender = self
            .store
            .find_participant(work_id, "Human")?
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("Work has no Human participant".into())
            })?;
        let message_id = MessageId::new();
        self.store.add_message(WorkMessage {
            id: message_id.clone(),
            work_id: work_id.clone(),
            sender_participant_id: sender.id,
            recipient_id: recipient.id.clone(),
            provider_profile_id: profile.id.clone(),
            service: profile.service,
            model: profile.model.clone(),
            execution_mode: profile.execution_mode,
            content: message.into(),
            created_at: Utc::now(),
        })?;
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
        which::which("claude")
            .map(|_| "CLI found".into())
            .map_err(|_| RoutingError::Transport("Claude Code CLI was not found in PATH".into()))
    }
}

pub struct ChatGptManualAdapter;

impl ProviderAdapter for ChatGptManualAdapter {
    fn service(&self) -> ProviderService {
        ProviderService::ChatGpt
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

fn repository_snapshot(worktree: &Path) -> String {
    let root = worktree.canonicalize().ok();
    let mut output = String::from("REPOSITORY ORIENTATION\n");
    for name in [
        "README.md",
        "README",
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "go.mod",
        "Package.swift",
    ] {
        let path = worktree.join(name);
        let Some(root) = root.as_ref() else { break };
        let Ok(resolved) = path.canonicalize() else {
            continue;
        };
        if !resolved.starts_with(root) || !resolved.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&resolved) else {
            continue;
        };
        let remaining = REPOSITORY_SNAPSHOT_LIMIT.saturating_sub(output.len());
        if remaining == 0 {
            break;
        }
        let content = bounded_to(&content, remaining.saturating_sub(name.len() + 8));
        let _ = write!(output, "\nFILE {name}\n{content}\n");
    }
    output.push_str("END_REPOSITORY_ORIENTATION\n");
    output
}

fn bounded_to(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
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
            Ok("available".into())
        }
    }

    #[test]
    fn routes_only_to_the_explicit_recipient_and_records_provenance() {
        let directory = TempDir::new().unwrap();
        std::fs::write(
            directory.path().join("README.md"),
            "# Routed project\nProvider-neutral coordination.",
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
        };
        let credentials = MemoryCredentialStore::new();
        let router = RoutingService::new(&store, &credentials, vec![&adapter]);

        router
            .send(&work.id, &recipient.id, "Review the completed change")
            .unwrap();

        assert_eq!(adapter.calls.lock().unwrap().len(), 1);
        let routed = adapter.calls.lock().unwrap()[0].1.clone();
        assert!(routed.contains("COMBE_WORK_CONTEXT\nversion: 2"));
        assert!(routed.contains("title: Routed work"));
        assert!(routed.contains("FILE README.md\n# Routed project"));
        assert!(routed.contains("ROUTED REQUEST\nReview the completed change"));
        let messages = store.messages(&work.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].recipient_id, recipient.id);
        assert_eq!(messages[0].provider_profile_id, profile.id);
        assert_eq!(messages[0].service, ProviderService::ChatGpt);
        let results = store.provider_results(&work.id).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, messages[0].id);
        assert_eq!(results[0].recipient_id, recipient.id);
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
        };
        let credentials = MemoryCredentialStore::new();
        let router = RoutingService::new(&store, &credentials, vec![&adapter]);

        assert!(matches!(
            router.send(&work.id, &recipient.id, "Run locally"),
            Err(RoutingError::MissingTransport(ProviderService::Ollama))
        ));
        assert!(adapter.calls.lock().unwrap().is_empty());
        assert!(store.messages(&work.id).unwrap().is_empty());
        assert!(store.provider_results(&work.id).unwrap().is_empty());
    }
}
