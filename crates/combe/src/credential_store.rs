#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

use combe_state::CredentialRef;

pub struct Credential(Vec<u8>);

impl Credential {
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("credential is not configured: {0}")]
    Missing(String),
    #[error("macOS Keychain operation failed")]
    Keychain,
}

pub trait CredentialStore: Send + Sync {
    fn get(&self, reference: &CredentialRef) -> Result<Credential, CredentialError>;
    fn set(&self, reference: &CredentialRef, credential: Credential)
    -> Result<(), CredentialError>;
    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError>;
}

pub struct KeychainCredentialStore {
    service: String,
}

impl KeychainCredentialStore {
    pub fn new() -> Self {
        Self {
            service: "com.randy.combe.provider-credential".into(),
        }
    }
}

impl CredentialStore for KeychainCredentialStore {
    fn get(&self, reference: &CredentialRef) -> Result<Credential, CredentialError> {
        security_framework::passwords::get_generic_password(&self.service, &reference.0)
            .map(Credential)
            .map_err(|_| CredentialError::Missing(reference.0.clone()))
    }

    fn set(
        &self,
        reference: &CredentialRef,
        credential: Credential,
    ) -> Result<(), CredentialError> {
        security_framework::passwords::set_generic_password(
            &self.service,
            &reference.0,
            credential.expose(),
        )
        .map_err(|_| CredentialError::Keychain)
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        security_framework::passwords::delete_generic_password(&self.service, &reference.0)
            .map_err(|_| CredentialError::Missing(reference.0.clone()))
    }
}

#[cfg(test)]
pub struct MemoryCredentialStore {
    values: Mutex<HashMap<String, Vec<u8>>>,
}

#[cfg(test)]
impl MemoryCredentialStore {
    pub fn new() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl CredentialStore for MemoryCredentialStore {
    fn get(&self, reference: &CredentialRef) -> Result<Credential, CredentialError> {
        self.values
            .lock()
            .expect("credential test store poisoned")
            .get(&reference.0)
            .cloned()
            .map(Credential)
            .ok_or_else(|| CredentialError::Missing(reference.0.clone()))
    }

    fn set(
        &self,
        reference: &CredentialRef,
        credential: Credential,
    ) -> Result<(), CredentialError> {
        self.values
            .lock()
            .expect("credential test store poisoned")
            .insert(reference.0.clone(), credential.0);
        Ok(())
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        self.values
            .lock()
            .expect("credential test store poisoned")
            .remove(&reference.0)
            .map(|_| ())
            .ok_or_else(|| CredentialError::Missing(reference.0.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_round_trip_uses_only_the_reference_as_identity() {
        let store = MemoryCredentialStore::new();
        let reference = CredentialRef("openai/test".into());
        store
            .set(&reference, Credential::new(b"sensitive-value".to_vec()))
            .unwrap();
        assert_eq!(store.get(&reference).unwrap().expose(), b"sensitive-value");
        store.delete(&reference).unwrap();
        assert!(matches!(
            store.get(&reference),
            Err(CredentialError::Missing(value)) if value == "openai/test"
        ));
    }
}
