use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl Request {
    pub fn new(operation: impl Into<String>, payload: Option<serde_json::Value>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation: operation.into(),
            payload,
        }
    }

    pub fn query(collection: impl Into<String>, filter: serde_json::Value) -> Self {
        Self::new(
            "query",
            Some(serde_json::json!({
                "collection": collection.into(),
                "filter": filter
            })),
        )
    }

    pub fn transact(operations: Vec<TransactOperation>) -> Self {
        Self::new(
            "transact",
            Some(serde_json::json!({
                "operations": operations
            })),
        )
    }

    pub fn hello(client: impl Into<String>, database: impl Into<String>) -> Self {
        Self::new(
            "hello",
            Some(serde_json::json!({
                "client": client.into(),
                "database": database.into(),
            })),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
}

impl Response {
    pub fn success(request_id: String, payload: Option<serde_json::Value>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            ok: Some(true),
            payload,
            error: None,
        }
    }

    pub fn error(request_id: String, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            ok: Some(false),
            payload: None,
            error: Some(ErrorResponse {
                code: code.into(),
                message: message.into(),
            }),
        }
    }

    pub fn is_success(&self) -> bool {
        self.ok.unwrap_or(false)
    }

    pub fn get_error_message(&self) -> Option<String> {
        self.error.as_ref().map(|e| e.message.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TransactOperation {
    #[serde(rename = "upsert")]
    Upsert {
        collection: String,
        #[serde(flatten)]
        document: serde_json::Value,
    },
    #[serde(rename = "delete")]
    Delete {
        collection: String,
        id: String,
    },
    #[serde(rename = "query")]
    Query {
        collection: String,
        filter: serde_json::Value,
    },
}

#[derive(Debug)]
pub struct RuntimeConfig {
    pub client_id: String,
    pub database_id: String,
    pub socket_path: PathBuf,
    pub launch_runtime: bool,
}

impl RuntimeConfig {
    pub fn for_combe() -> Self {
        Self {
            client_id: "combe".to_string(),
            database_id: "combe".to_string(),
            socket_path: Self::default_socket_path(),
            launch_runtime: true,
        }
    }

    fn default_socket_path() -> PathBuf {
        let data_dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        data_dir.join("combe").join("feltdb.sock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = Request::new("query", Some(serde_json::json!({"test": "value"})));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"operation\":\"query\""));
    }

    #[test]
    fn test_response_success() {
        let resp = Response::success(
            "req-123".to_string(),
            Some(serde_json::json!({"result": "ok"})),
        );
        assert!(resp.is_success());
        assert_eq!(resp.version, PROTOCOL_VERSION);
    }

    #[test]
    fn test_response_error() {
        let resp = Response::error("req-456".to_string(), "NOT_FOUND", "Entity not found");
        assert!(!resp.is_success());
        assert_eq!(resp.error.unwrap().code, "NOT_FOUND");
    }

    #[test]
    fn test_hello_request() {
        let req = Request::hello("combe", "combe");
        assert_eq!(req.operation, "hello");
        let payload = req.payload.unwrap();
        assert_eq!(payload["client"], "combe");
    }
}
