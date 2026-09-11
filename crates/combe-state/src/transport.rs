use crate::{Result, StateError};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024; // 10MB
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct SocketTransport {
    #[cfg(unix)]
    socket: std::os::unix::net::UnixStream,
    #[cfg(not(unix))]
    socket: std::marker::PhantomData<()>,
}

impl SocketTransport {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;

            let stream = UnixStream::connect(socket_path).map_err(|e| {
                StateError::FeltDbError(format!(
                    "Failed to connect to {}: {}",
                    socket_path.display(),
                    e
                ))
            })?;

            stream.set_read_timeout(Some(READ_TIMEOUT)).map_err(|e| {
                StateError::FeltDbError(format!("Failed to set read timeout: {}", e))
            })?;

            stream.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(|e| {
                StateError::FeltDbError(format!("Failed to set write timeout: {}", e))
            })?;

            Ok(Self { socket: stream })
        }

        #[cfg(not(unix))]
        {
            Err(StateError::FeltDbError(
                "Unix domain sockets only available on Unix".to_string(),
            ))
        }
    }

    pub fn send_message(&mut self, message: &Value) -> Result<()> {
        let json_bytes = serde_json::to_vec(message)?;

        if json_bytes.len() > MAX_MESSAGE_SIZE {
            return Err(StateError::FeltDbError(format!(
                "Message too large: {} bytes (max {})",
                json_bytes.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        let size = json_bytes.len() as u32;
        let size_bytes = size.to_le_bytes();

        #[cfg(unix)]
        {
            self.socket.write_all(&size_bytes).map_err(|e| {
                StateError::FeltDbError(format!("Failed to write message size: {}", e))
            })?;

            self.socket.write_all(&json_bytes).map_err(|e| {
                StateError::FeltDbError(format!("Failed to write message: {}", e))
            })?;
        }

        Ok(())
    }

    pub fn receive_message(&mut self) -> Result<Value> {
        #[cfg(unix)]
        {
            let mut size_bytes = [0u8; 4];
            self.socket.read_exact(&mut size_bytes).map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    StateError::FeltDbError("Runtime closed connection".to_string())
                } else {
                    StateError::FeltDbError(format!("Failed to read message size: {}", e))
                }
            })?;

            let size = u32::from_le_bytes(size_bytes) as usize;

            if size > MAX_MESSAGE_SIZE {
                return Err(StateError::FeltDbError(format!(
                    "Message size {} exceeds maximum {}",
                    size, MAX_MESSAGE_SIZE
                )));
            }

            let mut buffer = vec![0u8; size];
            self.socket.read_exact(&mut buffer).map_err(|e| {
                StateError::FeltDbError(format!("Failed to read message: {}", e))
            })?;

            serde_json::from_slice(&buffer).map_err(|e| {
                StateError::FeltDbError(format!("Malformed message from runtime: {}", e))
            })
        }

        #[cfg(not(unix))]
        {
            Err(StateError::FeltDbError(
                "Unix domain sockets only available on Unix".to_string(),
            ))
        }
    }
}

pub fn ensure_socket_dir(socket_path: &Path) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|e| StateError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    Ok(())
}

pub fn remove_stale_socket(socket_path: &Path) -> Result<()> {
    if socket_path.exists() {
        fs::remove_file(socket_path).map_err(|e| {
            if e.kind() != std::io::ErrorKind::NotFound {
                StateError::Io {
                    path: socket_path.to_path_buf(),
                    source: e,
                }
            } else {
                StateError::FeltDbError("Socket already removed".to_string())
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_framing() {
        let size = 1234u32;
        let bytes = size.to_le_bytes();
        let decoded = u32::from_le_bytes(bytes);
        assert_eq!(decoded, size);
    }

    #[test]
    fn test_max_message_size_validation() {
        let huge_json = Value::String("x".repeat(MAX_MESSAGE_SIZE + 1));
        let result = serde_json::to_vec(&huge_json);
        assert!(result.is_ok());
        assert!(result.unwrap().len() > MAX_MESSAGE_SIZE);
    }

    #[test]
    fn test_socket_dir_creation() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("subdir").join("test.sock");
        ensure_socket_dir(&socket_path).unwrap();
        assert!(socket_path.parent().unwrap().exists());
    }
}
