use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{Result, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptImportSource {
    pub id: String,
    pub provenance: String,
    pub display_name: String,
    pub imported_at: Timestamp,
    pub conversation_count: usize,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptAttachment {
    pub id: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptHistoryMessage {
    pub id: String,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    pub role: Option<String>,
    pub created_at: Option<Timestamp>,
    pub content: String,
    pub attachments: Vec<ChatGptAttachment>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptHistoryConversation {
    pub source_id: String,
    pub conversation_id: String,
    pub title: Option<String>,
    pub created_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
    pub messages: Vec<ChatGptHistoryMessage>,
    #[serde(default)]
    pub source_metadata: BTreeMap<String, String>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptImportResult {
    pub new_conversations: usize,
    pub updated_conversations: usize,
    pub new_messages: usize,
    pub duplicate_conversations: usize,
    pub skipped_conversations: usize,
}

pub trait ChatGptHistoryStore {
    fn import_chatgpt_source(
        &self,
        source: ChatGptImportSource,
        conversations: Vec<ChatGptHistoryConversation>,
    ) -> Result<ChatGptImportResult>;
    fn chatgpt_sources(&self) -> Result<Vec<ChatGptImportSource>>;
    fn chatgpt_conversations(
        &self,
        source_id: Option<&str>,
    ) -> Result<Vec<ChatGptHistoryConversation>>;
    fn delete_chatgpt_source(&self, source_id: &str) -> Result<()>;
}
