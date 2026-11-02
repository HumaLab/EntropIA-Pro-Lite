//! The client token: the OS credential store first and, only when this machine
//! has none, the owner-only file the app wrote next to its archive (D14).
//!
//! The app stores the token under `(service, account)` computed by
//! `automation::access::keyring_target`; the pair here must stay identical, so a
//! dev profile cannot read the real app's token and the real app cannot read a
//! profile's. When no credential store is available (or it has no entry) the app
//! writes `<data dir>/agent-tokens/<client id>.token` instead, and the connector
//! reads the same path. No agent configuration ever carries the token itself
//! (S4).
//!
//! On Unix a token file that group or others can read is refused rather than
//! trusted: the app writes it `0600`, so anything wider means someone else could
//! already have copied it (`credential_insecure`, D14).

use std::path::{Path, PathBuf};

use crate::relay::{ConnectorSource, TokenError};

/// Keyring service of the connector tokens, shared with the app.
pub const KEYRING_SERVICE: &str = "com.entropia.agent-connector";

/// Directory, under the data directory, holding the fallback tokens. The app
/// writes them there too (`automation::access::TOKENS_DIR`).
pub const TOKENS_DIR: &str = "agent-tokens";

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

/// The fallback token file of one client, under `data_dir` (which is already
/// profile-aware: see [`crate::cli::resolve_data_dir`]).
pub fn token_path(data_dir: &Path, client_id: &str) -> PathBuf {
    data_dir.join(TOKENS_DIR).join(format!("{client_id}.token"))
}

/// What the credential store had to say about one client id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringOutcome {
    /// The store holds this client's token.
    Token(String),
    /// The store has no entry for this client: the file is the fallback (D14).
    NoEntry,
    /// The store cannot be reached from this process (no Secret Service, no
    /// session bus): the file is still the fallback, and without one the error
    /// says the store was unreachable rather than "not paired".
    Unavailable(String),
    /// A store error that is not "unavailable": report it and never fall back,
    /// so a broken store does not silently read from disk.
    Failed(String),
}

/// The credential store step, injectable so tests never touch a real keyring.
pub trait CredentialStore: Send + Sync {
    fn read(&self, client_id: &str) -> KeyringOutcome;
}

/// The OS credential store under the profile's `(service, account)`.
pub struct OsCredentialStore {
    profile: Option<String>,
}

impl OsCredentialStore {
    pub fn new(profile: Option<&str>) -> Self {
        Self {
            profile: profile.map(str::to_string),
        }
    }
}

/// Whether a store failure means this machine has no usable credential store,
/// as opposed to a problem with one entry. Mirrors
/// `settings::is_credential_store_unavailable` in the app.
pub fn is_store_unavailable(error: &keyring::Error) -> bool {
    matches!(
        error,
        keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_)
    )
}

impl CredentialStore for OsCredentialStore {
    fn read(&self, client_id: &str) -> KeyringOutcome {
        let (service, account) = keyring_target(self.profile.as_deref(), client_id);
        let entry = match keyring::Entry::new(&service, &account) {
            Ok(entry) => entry,
            Err(error) if is_store_unavailable(&error) => {
                return KeyringOutcome::Unavailable(error.to_string())
            }
            Err(error) => {
                return KeyringOutcome::Failed(format!(
                    "no pude abrir el llavero del sistema ({service} / {account}): {error}"
                ))
            }
        };
        match entry.get_password() {
            Ok(token) => KeyringOutcome::Token(token),
            Err(keyring::Error::NoEntry) => KeyringOutcome::NoEntry,
            Err(error) if is_store_unavailable(&error) => {
                KeyringOutcome::Unavailable(error.to_string())
            }
            Err(error) => KeyringOutcome::Failed(format!(
                "no pude leer la credencial del llavero del sistema ({service} / {account}): {error}"
            )),
        }
    }
}

/// The production source: the credential store, the fallback file of one
/// profile, and that profile's discovery file.
pub struct KeyringSource {
    discovery: PathBuf,
    data_dir: PathBuf,
    store: Box<dyn CredentialStore>,
}

impl KeyringSource {
    pub fn new(
        discovery: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        profile: Option<&str>,
    ) -> Self {
        Self {
            discovery: discovery.into(),
            data_dir: data_dir.into(),
            store: Box::new(OsCredentialStore::new(profile)),
        }
    }

    /// The same lookup with a scripted store: tests use this, production never
    /// does.
    pub fn with_store(
        discovery: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        store: Box<dyn CredentialStore>,
    ) -> Self {
        Self {
            discovery: discovery.into(),
            data_dir: data_dir.into(),
            store,
        }
    }
}

impl ConnectorSource for KeyringSource {
    fn discovery_path(&self) -> PathBuf {
        self.discovery.clone()
    }

    fn token(&self, client_id: &str) -> Result<String, TokenError> {
        match self.store.read(client_id) {
            KeyringOutcome::Token(token) => Ok(token),
            KeyringOutcome::Failed(reason) => Err(TokenError::Failed(reason)),
            KeyringOutcome::NoEntry => read_token_file(&self.data_dir, client_id),
            KeyringOutcome::Unavailable(reason) => match read_token_file(&self.data_dir, client_id)
            {
                Err(TokenError::Missing) => Err(TokenError::StoreUnavailable(reason)),
                other => other,
            },
        }
    }
}

/// Read the fallback token. A missing file is [`TokenError::Missing`]; a file
/// group or others can read is [`TokenError::Insecure`] on Unix (D14).
fn read_token_file(data_dir: &Path, client_id: &str) -> Result<String, TokenError> {
    let path = token_path(data_dir, client_id);
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(TokenError::Missing)
        }
        Err(error) => {
            return Err(TokenError::Failed(format!(
                "no pude leer {}: {error}",
                path.display()
            )))
        }
    };
    // The mode comes from the open handle, not the path, so a swapped file
    // cannot pass the check with one mode and be read with another.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .map_err(|error| {
                TokenError::Failed(format!("no pude leer {}: {error}", path.display()))
            })?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(TokenError::Insecure(format!(
                "{} es legible por el grupo o por otros usuarios (modo {:03o})",
                path.display(),
                mode & 0o777
            )));
        }
    }
    let mut token = String::new();
    std::io::Read::read_to_string(&mut file, &mut token)
        .map_err(|error| TokenError::Failed(format!("no pude leer {}: {error}", path.display())))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(TokenError::Missing);
    }
    Ok(token.to_string())
}
