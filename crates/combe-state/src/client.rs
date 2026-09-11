use crate::protocol::{Request, Response, RuntimeConfig, TransactOperation};
use crate::{Result, StateError};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct FeltDbLocalClient {
    config: RuntimeConfig,
    _connection: Arc<Mutex<ClientConnection>>,
}

struct ClientConnection {
    socket_path: PathBuf,
    _connected: bool,
}

impl FeltDbLocalClient {
    pub fn connect(config: RuntimeConfig) -> Result<Self> {
        Self::ensure_runtime(&config)?;
        Self::perform_handshake(&config)?;

        let connection = ClientConnection {
            socket_path: config.socket_path.clone(),
            _connected: true,
        };

        Ok(Self {
            config,
            _connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn for_combe() -> Result<Self> {
        Self::connect(RuntimeConfig::for_combe())
    }

    fn ensure_runtime(config: &RuntimeConfig) -> Result<()> {
        if config.socket_path.exists() {
            return Ok(());
        }

        if !config.launch_runtime {
            return Err(StateError::FeltDbError(format!(
                "FeltDB runtime not available at {}",
                config.socket_path.display()
            )));
        }

        Self::launch_runtime(config)?;
        Self::wait_for_ready(config)?;

        Ok(())
    }

    fn launch_runtime(config: &RuntimeConfig) -> Result<()> {
        todo!("Launch bundled FeltDB local runtime for socket: {}", config.socket_path.display())
    }

    fn wait_for_ready(config: &RuntimeConfig) -> Result<()> {
        todo!("Wait for FeltDB runtime to be ready at socket: {}", config.socket_path.display())
    }

    fn perform_handshake(config: &RuntimeConfig) -> Result<()> {
        todo!("Perform HELLO/READY handshake with runtime")
    }

    fn send_request(&self, request: Request) -> Result<Response> {
        todo!("Send request over Unix domain socket and receive response")
    }

    pub fn query(
        &self,
        collection: &str,
        filter: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let request = Request::query(collection, filter);
        let response = self.send_request(request)?;

        if !response.is_success() {
            return Err(StateError::FeltDbError(
                response.get_error_message().unwrap_or_default(),
            ));
        }

        Ok(response.payload.unwrap_or(json!({})))
    }

    pub fn transact(&self, operations: Vec<TransactOperation>) -> Result<serde_json::Value> {
        let request = Request::transact(operations);
        let response = self.send_request(request)?;

        if !response.is_success() {
            return Err(StateError::FeltDbError(
                response.get_error_message().unwrap_or_default(),
            ));
        }

        Ok(response.payload.unwrap_or(json!({})))
    }

    pub fn upsert(
        &self,
        collection: &str,
        document: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let operation = TransactOperation::Upsert {
            collection: collection.to_string(),
            document,
        };
        self.transact(vec![operation])
    }

    pub fn delete(&self, collection: &str, id: &str) -> Result<()> {
        let operation = TransactOperation::Delete {
            collection: collection.to_string(),
            id: id.to_string(),
        };
        self.transact(vec![operation])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_config_for_combe() {
        let config = RuntimeConfig::for_combe();
        assert_eq!(config.client_id, "combe");
        assert_eq!(config.database_id, "combe");
        assert!(config.launch_runtime);
    }

    #[test]
    fn test_socket_path_in_data_dir() {
        let config = RuntimeConfig::for_combe();
        let path_str = config.socket_path.to_string_lossy();
        assert!(path_str.contains("combe"));
        assert!(path_str.contains("feltdb.sock"));
    }
}
