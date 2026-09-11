use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

id!(ProviderProfileId);
id!(RecipientId);
id!(MessageId);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialRef(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderService {
    Ollama,
    ClaudeCode,
    ChatGpt,
    OpenAiApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    LocalHttp,
    LocalCli,
    ExternalManual,
    HttpApi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub text_generation: bool,
    pub code_execution: bool,
    pub structured_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: ProviderProfileId,
    pub name: String,
    pub service: ProviderService,
    pub model: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub execution_mode: ExecutionMode,
    pub credential_ref: Option<CredentialRef>,
    pub endpoint: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipient {
    pub id: RecipientId,
    pub name: String,
    pub provider_profile_id: ProviderProfileId,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkMessage {
    pub id: MessageId,
    pub work_id: crate::WorkId,
    pub sender_participant_id: crate::ParticipantId,
    pub recipient_id: RecipientId,
    pub provider_profile_id: ProviderProfileId,
    pub service: ProviderService,
    pub model: Option<String>,
    pub execution_mode: ExecutionMode,
    pub content: String,
    pub created_at: crate::Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderResultStatus {
    Completed,
    ManualTransferRequired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResult {
    pub provider_profile_id: ProviderProfileId,
    pub model: Option<String>,
    pub content: String,
    pub status: ProviderResultStatus,
    pub usage: Option<ProviderUsage>,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutedProviderResult {
    pub message_id: MessageId,
    pub work_id: crate::WorkId,
    pub recipient_id: RecipientId,
    pub provider_profile_id: ProviderProfileId,
    pub service: ProviderService,
    pub model: Option<String>,
    pub execution_mode: ExecutionMode,
    pub result: ProviderResult,
    pub created_at: crate::Timestamp,
}

impl ProviderProfile {
    pub fn validate(&self) -> crate::Result<()> {
        if self.name.trim().is_empty() {
            return Err(crate::StateError::InvalidEntity(
                "provider profile name cannot be empty".into(),
            ));
        }
        let valid_mode = match self.service {
            ProviderService::Ollama => self.execution_mode == ExecutionMode::LocalHttp,
            ProviderService::ClaudeCode => self.execution_mode == ExecutionMode::LocalCli,
            ProviderService::ChatGpt => matches!(
                self.execution_mode,
                ExecutionMode::ExternalManual | ExecutionMode::HttpApi
            ),
            ProviderService::OpenAiApi => self.execution_mode == ExecutionMode::HttpApi,
        };
        if !valid_mode {
            return Err(crate::StateError::InvalidEntity(
                "execution mode does not match provider service".into(),
            ));
        }
        if matches!(
            self.service,
            ProviderService::Ollama | ProviderService::OpenAiApi
        ) && self.model.as_deref().is_none_or(str::is_empty)
            || self.service == ProviderService::ChatGpt
                && self.execution_mode == ExecutionMode::HttpApi
                && self.model.as_deref().is_none_or(str::is_empty)
        {
            return Err(crate::StateError::InvalidEntity(
                "provider profile requires an explicit model".into(),
            ));
        }
        if self.service == ProviderService::Ollama && self.endpoint.is_none() {
            return Err(crate::StateError::InvalidEntity(
                "Ollama profile requires an endpoint".into(),
            ));
        }
        if self.service == ProviderService::OpenAiApi && self.credential_ref.is_none() {
            return Err(crate::StateError::InvalidEntity(
                "OpenAI API profile requires a credential reference".into(),
            ));
        }
        Ok(())
    }
}
