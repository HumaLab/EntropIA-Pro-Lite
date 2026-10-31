//! The client token, read from the OS credential store.
//!
//! The app writes the token under `(service, account)` computed by
//! `automation::access::keyring_target`; the pair here must stay identical, so a
//! dev profile cannot read the real app's token and the real app cannot read a
//! profile's. No agent configuration ever carries the token itself (S4).
//!
//! A file fallback for machines without a credential store is not part of this
//! tranche (D9); if the store cannot be read, the relay answers
//! `credential_missing` with the store's error as the detail.

use std::path::PathBuf;

use crate::relay::{ConnectorSource, TokenError};

/// Keyring service of the connector tokens, shared with the app.
pub const KEYRING_SERVICE: &str = "com.entropia.agent-connector";

/// The keyring `(service, account)` of a client token.
pub fn keyring_target(profile: Option<&str>, client_id: &str) -> (String, String) {
    match profile {
        None => (KEYRING_SERVICE.to_string(), client_id.to_string()),
        Some(name) => (
            format!("{KEYRING_SERVICE} (dev profile)"),
            format!("{client_id}@dev-profile:{name}"),
        ),
    }
}

/// The production source: the OS keyring, and the discovery file of one profile.
pub struct KeyringSource {
    discovery: PathBuf,
    profile: Option<String>,
}

impl KeyringSource {
    pub fn new(discovery: impl Into<PathBuf>, profile: Option<&str>) -> Self {
        Self {
            discovery: discovery.into(),
            profile: profile.map(str::to_string),
        }
    }
}

impl ConnectorSource for KeyringSource {
    fn discovery_path(&self) -> PathBuf {
        self.discovery.clone()
    }

    fn token(&self, client_id: &str) -> Result<String, TokenError> {
        let (service, account) = keyring_target(self.profile.as_deref(), client_id);
        let entry = keyring::Entry::new(&service, &account).map_err(|error| {
            TokenError::Failed(format!(
                "no pude abrir el llavero del sistema ({service} / {account}): {error}"
            ))
        })?;
        match entry.get_password() {
            Ok(token) => Ok(token),
            Err(keyring::Error::NoEntry) => Err(TokenError::Missing),
            Err(error) => Err(TokenError::Failed(format!(
                "no pude leer la credencial del llavero del sistema ({service} / {account}): {error}"
            ))),
        }
    }
}
