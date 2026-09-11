use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};
use combe_state::{
    ChatGptAttachment, ChatGptHistoryConversation, ChatGptHistoryMessage, ChatGptImportSource,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ChatGptHistoryError {
    #[error("ChatGPT export could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("ChatGPT export JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ChatGPT export is unsafe: {0}")]
    Unsafe(String),
    #[error("ChatGPT export contains no conversation JSON files")]
    NoConversations,
    #[error("ChatGPT export extraction failed: {0}")]
    Extraction(String),
}

pub struct ChatGptAcquisition {
    pub source: ChatGptImportSource,
    pub conversations: Vec<ChatGptHistoryConversation>,
    pub empty_count: usize,
    pub attachment_count: usize,
}

pub fn acquire(path: &Path) -> Result<ChatGptAcquisition, ChatGptHistoryError> {
    if is_zip(path) {
        let directory = extract_zip(path)?;
        acquire_files(&conversation_files(directory.path())?, path)
    } else {
        acquire_files(&[path.to_path_buf()], path)
    }
}

pub fn acquire_paths(paths: &[PathBuf]) -> Result<ChatGptAcquisition, ChatGptHistoryError> {
    let Some(first) = paths.first() else {
        return Err(ChatGptHistoryError::NoConversations);
    };
    if paths.len() == 1 {
        return acquire(first);
    }
    if paths.iter().any(|path| is_zip(path)) {
        return Err(ChatGptHistoryError::Unsafe(
            "select one ZIP or one or more conversation JSON files".into(),
        ));
    }
    acquire_files(paths, first)
}

fn extract_zip(path: &Path) -> Result<TempDir, ChatGptHistoryError> {
    let entries = Command::new("/usr/bin/zipinfo")
        .args(["-1"])
        .arg(path)
        .output()?;
    if !entries.status.success() {
        return Err(ChatGptHistoryError::Extraction(
            String::from_utf8_lossy(&entries.stderr).trim().into(),
        ));
    }
    for entry in String::from_utf8_lossy(&entries.stdout).lines() {
        let entry = Path::new(entry);
        if entry.is_absolute()
            || entry
                .to_string_lossy()
                .split(['/', '\\'])
                .any(|component| component == "..")
            || entry.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ChatGptHistoryError::Unsafe(
                "an archive entry escapes its extraction directory".into(),
            ));
        }
    }
    let listing = Command::new("/usr/bin/unzip")
        .args(["-l"])
        .arg(path)
        .output()?;
    let total = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .rev()
        .find_map(|line| {
            line.split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
        })
        .ok_or_else(|| ChatGptHistoryError::Unsafe("archive size is unavailable".into()))?;
    if total > MAX_EXTRACTED_BYTES {
        return Err(ChatGptHistoryError::Unsafe(
            "the expanded archive exceeds 4 GiB".into(),
        ));
    }
    let directory = tempfile::tempdir()?;
    let output = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(path)
        .arg(directory.path())
        .output()?;
    if !output.status.success() {
        return Err(ChatGptHistoryError::Extraction(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(directory)
}

fn is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("zip"))
}

fn conversation_files(root: &Path) -> Result<Vec<PathBuf>, ChatGptHistoryError> {
    let canonical_root = root.canonicalize()?;
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if name.starts_with("conversations") && name.ends_with(".json") {
                let canonical = path.canonicalize()?;
                if canonical.starts_with(&canonical_root) {
                    files.push(canonical);
                }
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(ChatGptHistoryError::NoConversations);
    }
    Ok(files)
}

fn acquire_files(
    paths: &[PathBuf],
    display_path: &Path,
) -> Result<ChatGptAcquisition, ChatGptHistoryError> {
    let source_id = uuid::Uuid::new_v4().to_string();
    let mut conversations = Vec::new();
    for path in paths {
        let value: Value = serde_json::from_slice(&fs::read(path)?)?;
        match value {
            Value::Array(values) => {
                for value in values {
                    if let Some(conversation) = normalize_conversation(&source_id, &value) {
                        conversations.push(conversation);
                    }
                }
            }
            Value::Object(_) => {
                if let Some(conversation) = normalize_conversation(&source_id, &value) {
                    conversations.push(conversation);
                }
            }
            _ => {}
        }
    }
    if conversations.is_empty() {
        return Err(ChatGptHistoryError::NoConversations);
    }
    conversations.sort_by(|left, right| left.conversation_id.cmp(&right.conversation_id));
    conversations.dedup_by(|left, right| left.conversation_id == right.conversation_id);
    let empty_count = conversations
        .iter()
        .filter(|conversation| conversation.messages.is_empty())
        .count();
    let attachment_count = conversations
        .iter()
        .filter(|conversation| {
            conversation
                .messages
                .iter()
                .any(|message| !message.attachments.is_empty())
        })
        .count();
    let fingerprint = fingerprint(
        &conversations
            .iter()
            .map(|value| value.fingerprint.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    for conversation in &mut conversations {
        conversation.source_id.clone_from(&fingerprint);
    }
    let source = ChatGptImportSource {
        id: fingerprint.clone(),
        provenance: "chatgpt_export".into(),
        display_name: display_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("ChatGPT export")
            .into(),
        imported_at: Utc::now(),
        conversation_count: conversations.len(),
        fingerprint,
    };
    Ok(ChatGptAcquisition {
        source,
        conversations,
        empty_count,
        attachment_count,
    })
}

fn normalize_conversation(source_id: &str, value: &Value) -> Option<ChatGptHistoryConversation> {
    value.get("mapping")?.as_object()?;
    let conversation_id = string(value, &["id", "conversation_id"])?;
    let mut messages = value
        .get("mapping")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|mapping| mapping.values())
        .filter_map(normalize_message)
        .collect::<Vec<_>>();
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let title = string(value, &["title"]);
    let created_at = timestamp(value.get("create_time"));
    let updated_at = timestamp(value.get("update_time"));
    let source_metadata = value
        .as_object()
        .into_iter()
        .flat_map(|object| object.iter())
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "id" | "conversation_id" | "title" | "create_time" | "update_time" | "mapping"
            )
        })
        .map(|(key, value)| (key.clone(), value.to_string()))
        .collect();
    let fingerprint_value = serde_json::json!({
        "id": conversation_id,
        "title": title,
        "created_at": created_at,
        "updated_at": updated_at,
        "messages": messages,
        "source_metadata": source_metadata,
    });
    Some(ChatGptHistoryConversation {
        source_id: source_id.into(),
        conversation_id,
        title,
        created_at,
        updated_at,
        messages,
        source_metadata,
        fingerprint: fingerprint(&fingerprint_value.to_string()),
    })
}

fn normalize_message(node: &Value) -> Option<ChatGptHistoryMessage> {
    let message = node.get("message")?;
    if message.is_null() {
        return None;
    }
    let id = string(message, &["id"]).or_else(|| string(node, &["id"]))?;
    let parent_id = string(node, &["parent"]);
    let child_ids = node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let role = message
        .pointer("/author/role")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let created_at = timestamp(message.get("create_time"));
    let content = message_content(message.get("content"));
    let attachments = attachment_metadata(message);
    let fingerprint_value = serde_json::json!({
        "id": id,
        "parent": parent_id,
        "children": child_ids,
        "role": role,
        "created_at": created_at,
        "content": content,
        "attachments": attachments,
    });
    Some(ChatGptHistoryMessage {
        id,
        parent_id,
        child_ids,
        role,
        created_at,
        content,
        attachments,
        fingerprint: fingerprint(&fingerprint_value.to_string()),
    })
}

fn message_content(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return text.into();
    }
    content
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| match part {
            Value::String(value) => Some(value.clone()),
            Value::Object(value) => value.get("text").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn attachment_metadata(message: &Value) -> Vec<ChatGptAttachment> {
    fn visit(
        value: &Value,
        found: &mut BTreeSet<(Option<String>, Option<String>, Option<String>)>,
    ) {
        match value {
            Value::Array(values) => values.iter().for_each(|value| visit(value, found)),
            Value::Object(values) => {
                let id = values
                    .get("file_id")
                    .or_else(|| values.get("asset_pointer"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let name = values
                    .get("name")
                    .or_else(|| values.get("filename"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let kind = values
                    .get("content_type")
                    .or_else(|| values.get("mime_type"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if id.is_some() || name.is_some() {
                    found.insert((id, name, kind));
                }
                values.values().for_each(|value| visit(value, found));
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    visit(message, &mut found);
    found
        .into_iter()
        .map(|(id, name, kind)| ChatGptAttachment { id, name, kind })
        .collect()
}

fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

fn timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let seconds = value?.as_f64()?;
    let whole = seconds.trunc() as i64;
    let nanos = ((seconds.fract().abs()) * 1_000_000_000.0) as u32;
    DateTime::from_timestamp(whole, nanos)
}

fn fingerprint(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn matches(conversation: &ChatGptHistoryConversation, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || conversation
            .title
            .as_deref()
            .is_some_and(|title| title.to_lowercase().contains(&query))
        || conversation
            .messages
            .iter()
            .any(|message| message.content.to_lowercase().contains(&query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normalizes_branches_timestamps_attachments_and_fingerprints() {
        let value = serde_json::json!({
            "id": "conversation-1",
            "title": "AuthPort",
            "create_time": 1_700_000_000.25,
            "mapping": {
                "node-1": {
                    "id": "node-1",
                    "parent": null,
                    "children": ["node-2", "node-3"],
                    "message": {
                        "id": "message-1",
                        "author": { "role": "user" },
                        "create_time": 1_700_000_001.0,
                        "content": { "content_type": "multimodal_text", "parts": ["hello", { "asset_pointer": "file-1", "name": "design.png" }] }
                    }
                }
            }
        });
        let conversation = normalize_conversation("source", &value).unwrap();
        assert_eq!(conversation.conversation_id, "conversation-1");
        assert_eq!(conversation.messages[0].child_ids, ["node-2", "node-3"]);
        assert_eq!(conversation.messages[0].content, "hello");
        assert_eq!(
            conversation.messages[0].attachments[0].name.as_deref(),
            Some("design.png")
        );
        assert_eq!(conversation.fingerprint.len(), 64);
        assert_eq!(conversation.messages[0].fingerprint.len(), 64);
    }

    #[test]
    fn search_matches_title_or_message_content() {
        let value = serde_json::json!({
            "id": "conversation-1",
            "title": "Architecture",
            "mapping": {
                "node-1": { "message": { "id": "message-1", "content": { "parts": ["provider routing"] } } }
            }
        });
        let conversation = normalize_conversation("source", &value).unwrap();
        assert!(matches(&conversation, "architecture"));
        assert!(matches(&conversation, "ROUTING"));
        assert!(!matches(&conversation, "unrelated"));
    }

    #[test]
    fn acquisition_discovers_array_exports_and_has_stable_source_identity() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(
            file,
            "{}",
            serde_json::json!([{
                "id": "conversation-1",
                "title": "Architecture",
                "mapping": {
                    "node-1": { "message": { "id": "message-1", "content": { "parts": ["provider routing"] } } }
                }
            }])
        )
        .unwrap();
        let first = acquire(file.path()).unwrap();
        let second = acquire(file.path()).unwrap();
        assert_eq!(first.source.id, second.source.id);
        assert_eq!(first.source.fingerprint, second.source.fingerprint);
        assert_eq!(first.conversations[0].source_id, first.source.id);
        assert_eq!(first.conversations.len(), 1);
    }

    #[test]
    fn acquisition_combines_numbered_files_and_preserves_empty_conversations() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("conversations-000.json");
        let second = directory.path().join("conversations-001.json");
        fs::write(
            &first,
            serde_json::json!([{"id":"one","mapping":{}}]).to_string(),
        )
        .unwrap();
        fs::write(
            &second,
            serde_json::json!([{"id":"two","title":"Second","mapping":{}}]).to_string(),
        )
        .unwrap();
        let acquisition = acquire_paths(&[first, second]).unwrap();
        assert_eq!(acquisition.conversations.len(), 2);
        assert_eq!(acquisition.empty_count, 2);
    }

    #[test]
    fn malformed_json_and_non_conversation_objects_are_rejected() {
        let mut malformed = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        malformed.write_all(b"{").unwrap();
        assert!(matches!(
            acquire(malformed.path()),
            Err(ChatGptHistoryError::Json(_))
        ));
        let mut unrelated = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        unrelated.write_all(br#"{"id":"account-record"}"#).unwrap();
        assert!(matches!(
            acquire(unrelated.path()),
            Err(ChatGptHistoryError::NoConversations)
        ));
    }
}
