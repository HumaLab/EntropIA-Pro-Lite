//! Who may talk to the agent endpoint, and with which token.
//!
//! The state lives in `agent-access.json` next to the archive, never in SQLite:
//! it is per machine (a paired agent on one computer means nothing on another),
//! so it must not sync, and keeping it out of the schema needs no migration.
//! The file holds only the SHA-256 of each token. The token itself goes to the
//! OS keyring under [`KEYRING_SERVICE`], where the connector reads it; no agent
//! configuration ever contains it.
//!
//! A machine with no usable credential store falls back to one file per client
//! under [`TOKENS_DIR`] (D14): the directory is `0700` and the file `0600` on
//! Unix, and the record names which of the two places holds the token. Any other
//! keyring error still fails the pairing, so a broken store never silently
//! downgrades to disk.
//!
//! Revocation drops the record first and the stored token second, so a token
//! copied out of the keyring or the file before revocation stops working the
//! moment the record is gone, whether or not the delete succeeds.
//!
//! Neither the access file, nor the discovery file, nor the token directory may
//! ever enter a backup or the asset sweep: they hold token hashes and secrets,
//! not archive content (`backup.rs`, `asset_sweep.rs`).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// File name of the access state, in the data directory.
pub const ACCESS_FILE: &str = "agent-access.json";

/// Directory, under the data directory, that holds the token files of clients
/// whose machine has no usable credential store (D14). The connector computes
/// the same path (`crates/entropia-agent-connector/src/keyring_source.rs`).
pub const TOKENS_DIR: &str = "agent-tokens";

/// Keyring service of the connector tokens. The connector computes the same
/// `(service, account)` pair (`crates/entropia-agent-connector`); change both or
/// neither.
pub const KEYRING_SERVICE: &str = "com.entropia.agent-connector";

/// Prefix of every token, so a leaked one is recognizable in a secret scanner.
pub const TOKEN_PREFIX: &str = "eat_";

const ACCESS_FILE_VERSION: u32 = 1;
const NAME_MAX_CHARS: usize = 64;
/// `last_used_at` is written to disk at most this often per client.
const LAST_USED_PERSIST_SECS: i64 = 60;

/// What a client may do. Only `Read` is granted in this release; the rest are
/// declared so the file format and the tool contract already name them. `Web`
/// (Navegador browsing, network egress) is apart from `Read` by owner decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    LocalWrite,
    RemoteSpend,
    ExternalWrite,
    Admin,
    Web,
}

/// Where a client's token is stored. Non-secret: it is part of the record and
/// of what Settings › Asistentes shows (D14). `serde(default)` is what lets an
/// access file written before the file fallback load unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenStore {
    #[default]
    Keyring,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientRecord {
    id: String,
    name: String,
    scopes: Vec<Scope>,
    token_sha256: String,
    #[serde(default)]
    token_store: TokenStore,
    created_at: i64,
    #[serde(default)]
    last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccessState {
    version: u32,
    enabled: bool,
    clients: Vec<ClientRecord>,
}

impl Default for AccessState {
    fn default() -> Self {
        Self {
            version: ACCESS_FILE_VERSION,
            enabled: false,
            clients: Vec::new(),
        }
    }
}

/// A client as the UI sees it: never the token or its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub id: String,
    pub name: String,
    pub scopes: Vec<Scope>,
    /// Whether the token lives in the keyring or in the owner-only file.
    pub token_store: TokenStore,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessView {
    pub enabled: bool,
    pub clients: Vec<ClientView>,
}

/// The authenticated caller of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub client_id: String,
    pub name: String,
    pub scopes: Vec<Scope>,
}

impl Principal {
    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }
}

/// Where tokens are kept. The keyring in the app; an in-memory map in tests,
/// so no test ever touches the real credential store.
pub trait SecretStore: Send + Sync {
    fn set(&self, client_id: &str, token: &str) -> Result<(), String>;
    fn delete(&self, client_id: &str) -> Result<(), String>;
}

/// The keyring `(service, account)` of a client token. A dev profile gets its
/// own service, so a profile can neither read nor replace a real client's token.
pub fn keyring_target(profile: Option<&str>, client_id: &str) -> (String, String) {
    match profile {
        None => (KEYRING_SERVICE.to_string(), client_id.to_string()),
        Some(name) => (
            format!("{KEYRING_SERVICE} (dev profile)"),
            format!("{client_id}@dev-profile:{name}"),
        ),
    }
}

pub struct KeyringSecretStore {
    profile: Option<String>,
}

impl KeyringSecretStore {
    pub fn new(profile: Option<&str>) -> Self {
        Self {
            profile: profile.map(str::to_string),
        }
    }

    fn entry(&self, client_id: &str) -> Result<keyring::Entry, String> {
        let (service, account) = keyring_target(self.profile.as_deref(), client_id);
        // The entry can itself report an unavailable store, and that error has
        // to carry the marker the fallback looks for (D14).
        keyring::Entry::new(&service, &account).map_err(|e| describe_keyring_error("open", &e))
    }
}

fn describe_keyring_error(action: &str, error: &keyring::Error) -> String {
    let message =
        format!("Could not {action} the assistant token in the system credential store: {error}");
    if crate::settings::is_credential_store_unavailable(error) {
        format!(
            "{}: {message}",
            crate::settings::CREDENTIAL_STORE_UNAVAILABLE
        )
    } else {
        message
    }
}

impl SecretStore for KeyringSecretStore {
    fn set(&self, client_id: &str, token: &str) -> Result<(), String> {
        self.entry(client_id)?
            .set_password(token)
            .map_err(|e| describe_keyring_error("store", &e))
    }

    fn delete(&self, client_id: &str) -> Result<(), String> {
        match self.entry(client_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(describe_keyring_error("delete", &e)),
        }
    }
}

/// The file that holds a client's token when this machine has no credential
/// store (D14). Named after the client id, so a leftover file for a record that
/// no longer exists authenticates nothing.
pub fn token_path(dir: &Path, client_id: &str) -> PathBuf {
    dir.join(TOKENS_DIR).join(format!("{client_id}.token"))
}

/// Whether a store error means this machine has no usable credential store
/// (D14). Only then may a token fall back to a file; a store that refuses one
/// entry must refuse the pairing instead.
fn is_store_unavailable(error: &str) -> bool {
    error.starts_with(crate::settings::CREDENTIAL_STORE_UNAVAILABLE)
}

/// Create `dir` (or repair an existing one) readable by its owner only.
fn create_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Could not restrict {}: {e}", dir.display()))?;
    }
    Ok(())
}

fn write_token_file(dir: &Path, client_id: &str, token: &str) -> Result<(), String> {
    create_private_dir(&dir.join(TOKENS_DIR))?;
    write_atomic(&token_path(dir, client_id), token.as_bytes())
}

fn delete_token_file(dir: &Path, client_id: &str) -> Result<(), String> {
    let path = token_path(dir, client_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Could not delete the assistant token file {}: {error}",
            path.display()
        )),
    }
}

pub struct AgentAccess {
    path: PathBuf,
    /// The data directory: the fallback token files live under it, so a dev
    /// profile keeps its own (its data directory is the profile's).
    dir: PathBuf,
    state: Mutex<AccessState>,
    secrets: Arc<dyn SecretStore>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

/// 244 random bits from the OS generator (two v4 UUIDs), hex-encoded.
fn mint_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn view(record: &ClientRecord) -> ClientView {
    ClientView {
        id: record.id.clone(),
        name: record.name.clone(),
        scopes: record.scopes.clone(),
        token_store: record.token_store,
        created_at: record.created_at,
        last_used_at: record.last_used_at,
    }
}

/// Write `bytes` to `path` through a sibling temp file and a rename, so a crash
/// never leaves half a file. Readable by the owner only on Unix. Shared with
/// [`super::server`], which writes the discovery file the same way.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| {
        format!(
            "Could not create a temporary file in {}: {e}",
            dir.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Could not restrict {}: {e}", tmp.path().display()))?;
    }
    use std::io::Write;
    tmp.write_all(bytes)
        .and_then(|_| tmp.as_file().sync_all())
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    tmp.persist(path)
        .map_err(|e| format!("Could not replace {}: {}", path.display(), e.error))?;
    Ok(())
}

impl AgentAccess {
    /// Load the access state from `dir`. A missing file is a disabled endpoint
    /// with no clients. An unreadable one is set aside (renamed with a
    /// `.corrupt` suffix) and replaced by that same empty state: failing closed
    /// costs a re-pairing, never access.
    pub fn load(dir: &Path, secrets: Arc<dyn SecretStore>) -> Self {
        let path = dir.join(ACCESS_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<AccessState>(&bytes) {
                Ok(state) if state.version == ACCESS_FILE_VERSION => state,
                outcome => {
                    let reason = match outcome {
                        Ok(state) => format!("unknown version {}", state.version),
                        Err(e) => e.to_string(),
                    };
                    let aside = path.with_extension(format!("json.corrupt-{}", now_secs()));
                    log::warn!(
                        target: "automation",
                        "{} is unreadable ({reason}); moved to {} and starting disabled",
                        path.display(),
                        aside.display()
                    );
                    let _ = std::fs::rename(&path, &aside);
                    AccessState::default()
                }
            },
            Err(_) => AccessState::default(),
        };
        Self {
            path,
            dir: dir.to_path_buf(),
            state: Mutex::new(state),
            secrets,
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AccessState>, String> {
        self.state
            .lock()
            .map_err(|_| "agent access state lock is poisoned".to_string())
    }

    fn persist(&self, state: &AccessState) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|e| format!("Could not serialize agent access state: {e}"))?;
        write_atomic(&self.path, &bytes)
    }

    pub fn view(&self) -> Result<AccessView, String> {
        let state = self.lock()?;
        Ok(AccessView {
            enabled: state.enabled,
            clients: state.clients.iter().map(view).collect(),
        })
    }

    /// Store a new token in the keyring, and only when this machine has no
    /// usable credential store, in the owner-only file (D14).
    fn store_token(&self, client_id: &str, token: &str) -> Result<TokenStore, String> {
        match self.secrets.set(client_id, token) {
            Ok(()) => Ok(TokenStore::Keyring),
            Err(error) if is_store_unavailable(&error) => {
                write_token_file(&self.dir, client_id, token)?;
                Ok(TokenStore::File)
            }
            Err(error) => Err(error),
        }
    }

    /// Remove the stored token of a client whose record is already gone. A file
    /// copy is always removed: a fallback may have happened in an earlier run.
    /// A keyring entry is only deleted when it could exist, so a machine without
    /// a store does not turn a complete revocation into an error.
    fn delete_token(&self, client_id: &str, store: TokenStore) -> Result<(), String> {
        let mut failures: Vec<String> = Vec::new();
        if store == TokenStore::Keyring {
            if let Err(error) = self.secrets.delete(client_id) {
                failures.push(error);
            }
        }
        if let Err(error) = delete_token_file(&self.dir, client_id) {
            failures.push(error);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.lock().map(|s| s.enabled).unwrap_or(false)
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut state = self.lock()?;
        let mut next = state.clone();
        next.enabled = enabled;
        self.persist(&next)?;
        *state = next;
        Ok(())
    }

    /// Pair a new client: mint its token into the secret store, then record its
    /// hash. Returns the client without the token; the connector reads the token
    /// from the store by client id.
    pub fn create_client(&self, name: &str) -> Result<ClientView, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > NAME_MAX_CHARS {
            return Err(format!(
                "invalid_client_name: the assistant name must be 1 to {NAME_MAX_CHARS} characters"
            ));
        }
        if name.chars().any(char::is_control) {
            return Err("invalid_client_name: the assistant name has control characters".into());
        }
        let mut state = self.lock()?;
        let record = ClientRecord {
            id: format!("cl_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            name: name.to_string(),
            scopes: vec![Scope::Read],
            token_sha256: String::new(),
            token_store: TokenStore::Keyring,
            created_at: now_secs(),
            last_used_at: None,
        };
        let token = mint_token();
        let token_store = self.store_token(&record.id, &token)?;
        let record = ClientRecord {
            token_sha256: sha256_hex(&token),
            token_store,
            ..record
        };
        let mut next = state.clone();
        next.clients.push(record.clone());
        if let Err(error) = self.persist(&next) {
            // Leave no orphan token behind a client that was never recorded.
            let _ = self.delete_token(&record.id, record.token_store);
            return Err(error);
        }
        *state = next;
        Ok(view(&record))
    }

    /// Revoke a client. `Ok(false)` when there was no such client. The record
    /// goes first: from that moment its token, wherever a copy of it lives, is
    /// refused. A failed store delete is reported after that.
    pub fn revoke_client(&self, client_id: &str) -> Result<bool, String> {
        let mut state = self.lock()?;
        let Some(record) = state.clients.iter().find(|c| c.id == client_id).cloned() else {
            return Ok(false);
        };
        let mut next = state.clone();
        next.clients.retain(|c| c.id != client_id);
        self.persist(&next)?;
        *state = next;
        drop(state);
        self.delete_token(&record.id, record.token_store)
            .map(|()| true)
            .map_err(|e| format!("revoked, but the token copy could not be removed: {e}"))
    }

    /// Replace a client's scopes. `create_client` always grants [`Scope::Read`],
    /// so this is how a test builds the "paired but not allowed to read" case the
    /// tool contract has to refuse.
    #[cfg(test)]
    pub(crate) fn set_scopes(&self, client_id: &str, scopes: Vec<Scope>) -> Result<(), String> {
        let mut state = self.lock()?;
        let mut next = state.clone();
        let record = next
            .clients
            .iter_mut()
            .find(|client| client.id == client_id)
            .ok_or_else(|| format!("no client {client_id}"))?;
        record.scopes = scopes;
        self.persist(&next)?;
        *state = next;
        Ok(())
    }

    /// The client a bearer token belongs to, if the endpoint is enabled and the
    /// token matches a live record.
    pub fn authenticate(&self, token: &str) -> Option<Principal> {
        if !token.starts_with(TOKEN_PREFIX) {
            return None;
        }
        let digest = sha256_hex(token);
        let mut state = self.lock().ok()?;
        if !state.enabled {
            return None;
        }
        let now = now_secs();
        let index = state
            .clients
            .iter()
            .position(|c| c.token_sha256 == digest)?;
        let client = &mut state.clients[index];
        let principal = Principal {
            client_id: client.id.clone(),
            name: client.name.clone(),
            scopes: client.scopes.clone(),
        };
        let stale = client
            .last_used_at
            .is_none_or(|t| now - t >= LAST_USED_PERSIST_SECS);
        if stale {
            client.last_used_at = Some(now);
            let snapshot = state.clone();
            if let Err(error) = self.persist(&snapshot) {
                log::warn!(target: "automation", "could not record assistant use: {error}");
            }
        }
        Some(principal)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory secret store that can be told to fail: either the way a machine
    /// with no credential store fails (D14, which allows the file fallback) or
    /// any other way (which must refuse the pairing).
    #[derive(Default)]
    pub(crate) struct MemorySecrets {
        pub(crate) tokens: Mutex<HashMap<String, String>>,
        /// This machine has no usable credential store.
        pub(crate) unavailable: bool,
        /// A keyring error that is not `credential_store_unavailable`.
        pub(crate) fail_set: bool,
        pub(crate) fail_delete: bool,
    }

    impl MemorySecrets {
        pub(crate) fn token(&self, id: &str) -> Option<String> {
            self.tokens.lock().unwrap().get(id).cloned()
        }
    }

    impl SecretStore for MemorySecrets {
        fn set(&self, id: &str, token: &str) -> Result<(), String> {
            if self.unavailable {
                return Err(format!(
                    "{}: no hay llavero del sistema",
                    crate::settings::CREDENTIAL_STORE_UNAVAILABLE
                ));
            }
            if self.fail_set {
                return Err("Could not store the assistant token: access denied".into());
            }
            self.tokens.lock().unwrap().insert(id.into(), token.into());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            if self.fail_delete {
                return Err("delete refused".into());
            }
            self.tokens.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn setup() -> (tempfile::TempDir, Arc<MemorySecrets>, AgentAccess) {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let access = AgentAccess::load(dir.path(), secrets.clone());
        (dir, secrets, access)
    }

    fn file(dir: &tempfile::TempDir) -> String {
        std::fs::read_to_string(dir.path().join(ACCESS_FILE)).unwrap_or_default()
    }

    #[test]
    fn a_fresh_archive_starts_disabled_with_no_clients_and_writes_nothing() {
        let (dir, _, access) = setup();
        assert_eq!(
            access.view().unwrap(),
            AccessView {
                enabled: false,
                clients: vec![]
            }
        );
        assert!(!dir.path().join(ACCESS_FILE).exists());
    }

    #[test]
    fn a_new_client_gets_read_only_scope_and_its_token_goes_to_the_store_not_the_file() {
        let (dir, secrets, access) = setup();
        let client = access.create_client("  Claude Code ").unwrap();
        assert_eq!(client.name, "Claude Code");
        assert_eq!(client.scopes, vec![Scope::Read]);
        assert!(client.id.starts_with("cl_"));
        let token = secrets.token(&client.id).expect("token stored");
        assert!(token.starts_with(TOKEN_PREFIX));
        assert_eq!(token.len(), TOKEN_PREFIX.len() + 64);
        let on_disk = file(&dir);
        assert!(!on_disk.contains(&token), "token must never reach the file");
        assert!(on_disk.contains(&sha256_hex(&token)));
    }

    #[test]
    fn the_view_never_exposes_a_token_or_its_hash() {
        let (_dir, secrets, access) = setup();
        let client = access.create_client("Codex").unwrap();
        let token = secrets.token(&client.id).unwrap();
        let json = serde_json::to_string(&access.view().unwrap()).unwrap();
        assert!(!json.contains(&token));
        assert!(!json.contains(&sha256_hex(&token)));
        assert!(
            !json.contains("tokenSha256"),
            "the hash is not for the UI either: {json}"
        );
        assert!(
            json.contains("\"tokenStore\":\"keyring\""),
            "the storage kind is the only token field the panel reads: {json}"
        );
    }

    #[test]
    fn invalid_names_are_refused_and_change_nothing() {
        let (dir, secrets, access) = setup();
        for name in ["", "   ", &"x".repeat(65), "a\u{7}b"] {
            let error = access.create_client(name).unwrap_err();
            assert!(error.starts_with("invalid_client_name"), "{error}");
        }
        assert!(access.view().unwrap().clients.is_empty());
        assert!(secrets.tokens.lock().unwrap().is_empty());
        assert!(!dir.path().join(ACCESS_FILE).exists());
    }

    #[test]
    fn a_store_failure_that_is_not_unavailable_creates_no_client_and_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets {
            fail_set: true,
            ..Default::default()
        });
        let access = AgentAccess::load(dir.path(), secrets);
        let error = access.create_client("Pi").unwrap_err();
        assert!(
            !error.starts_with(crate::settings::CREDENTIAL_STORE_UNAVAILABLE),
            "only an unavailable store falls back: {error}"
        );
        assert!(access.view().unwrap().clients.is_empty());
        assert!(!dir.path().join(ACCESS_FILE).exists());
        assert!(
            !dir.path().join(TOKENS_DIR).exists(),
            "a refused pairing writes no token file (D14)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn without_a_credential_store_the_token_goes_to_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets {
            unavailable: true,
            ..Default::default()
        });
        let access = AgentAccess::load(dir.path(), secrets.clone());
        let client = access.create_client("Claude Code").unwrap();
        assert_eq!(client.token_store, TokenStore::File);

        let path = token_path(dir.path(), &client.id);
        let token = std::fs::read_to_string(&path).expect("the token file exists");
        assert!(token.starts_with(TOKEN_PREFIX));
        assert_eq!(token.len(), TOKEN_PREFIX.len() + 64);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        // The record keeps only the hash and the store it used.
        let on_disk = file(&dir);
        assert!(
            !on_disk.contains(&token),
            "the token never reaches the file"
        );
        assert!(on_disk.contains(&sha256_hex(&token)));
        assert!(on_disk.contains("\"token_store\": \"file\""), "{on_disk}");
        assert_eq!(
            secrets.token(&client.id),
            None,
            "nothing went to the keyring"
        );

        // The endpoint and the panel see the same state as with a keyring.
        access.set_enabled(true).unwrap();
        assert_eq!(access.authenticate(&token).unwrap().client_id, client.id);
        let json = serde_json::to_string(&access.view().unwrap()).unwrap();
        assert!(!json.contains(&token));
        assert!(json.contains("\"tokenStore\":\"file\""), "{json}");
    }

    #[cfg(unix)]
    #[test]
    fn revoking_a_file_client_removes_the_token_file() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets {
            unavailable: true,
            // A store that has no entry to delete must not fail the revocation.
            fail_delete: true,
            ..Default::default()
        });
        let access = AgentAccess::load(dir.path(), secrets);
        access.set_enabled(true).unwrap();
        let client = access.create_client("Codex").unwrap();
        let path = token_path(dir.path(), &client.id);
        let token = std::fs::read_to_string(&path).unwrap();

        assert!(access.revoke_client(&client.id).unwrap());
        assert!(!path.exists(), "the live token copy is gone");
        assert_eq!(access.authenticate(&token), None);
        assert!(access.view().unwrap().clients.is_empty());
    }

    #[test]
    fn an_access_file_from_before_the_fallback_loads_as_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let access = AgentAccess::load(dir.path(), secrets.clone());
        let client = access.create_client("OpenCode").unwrap();
        let token = secrets.token(&client.id).unwrap();

        // Rewrite the file the way the previous version wrote it: no tokenStore.
        let path = dir.path().join(ACCESS_FILE);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["clients"][0]
            .as_object_mut()
            .unwrap()
            .remove("token_store");
        std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        access.set_enabled(true).unwrap();
        let reloaded = AgentAccess::load(dir.path(), secrets);
        let view = reloaded.view().unwrap();
        assert_eq!(view.clients[0].token_store, TokenStore::Keyring);
        assert_eq!(reloaded.authenticate(&token).unwrap().client_id, client.id);
    }

    #[test]
    fn authentication_needs_the_endpoint_enabled_and_a_live_token() {
        let (_dir, secrets, access) = setup();
        let client = access.create_client("OpenCode").unwrap();
        let token = secrets.token(&client.id).unwrap();
        assert_eq!(access.authenticate(&token), None, "disabled endpoint");
        access.set_enabled(true).unwrap();
        let principal = access.authenticate(&token).expect("valid token");
        assert_eq!(principal.client_id, client.id);
        assert!(principal.has(Scope::Read));
        assert!(!principal.has(Scope::LocalWrite));
        assert_eq!(access.authenticate("eat_wrong"), None);
        assert_eq!(access.authenticate(""), None);
        assert_eq!(
            access.authenticate(&sha256_hex(&token)),
            None,
            "hash is not a token"
        );
    }

    #[test]
    fn a_revoked_token_is_refused_even_when_a_copy_survives() {
        let (_dir, secrets, access) = setup();
        access.set_enabled(true).unwrap();
        let client = access.create_client("Claude Code").unwrap();
        let copied = secrets.token(&client.id).unwrap();
        assert!(access.revoke_client(&client.id).unwrap());
        assert_eq!(access.authenticate(&copied), None);
        assert_eq!(secrets.token(&client.id), None);
        assert!(
            !access.revoke_client(&client.id).unwrap(),
            "second revoke is a no-op"
        );
    }

    #[test]
    fn revocation_holds_when_the_keyring_delete_fails() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let access = AgentAccess::load(dir.path(), secrets.clone());
        access.set_enabled(true).unwrap();
        let client = access.create_client("Pi").unwrap();
        let copied = secrets.token(&client.id).unwrap();
        let failing = Arc::new(MemorySecrets {
            fail_delete: true,
            ..Default::default()
        });
        let access = AgentAccess::load(dir.path(), failing);
        let error = access.revoke_client(&client.id).unwrap_err();
        assert!(error.starts_with("revoked, but"), "{error}");
        assert_eq!(access.authenticate(&copied), None);
        assert!(access.view().unwrap().clients.is_empty());
    }

    #[test]
    fn state_survives_a_reload() {
        let (dir, secrets, access) = setup();
        access.set_enabled(true).unwrap();
        let client = access.create_client("Codex").unwrap();
        let token = secrets.token(&client.id).unwrap();
        let reloaded = AgentAccess::load(dir.path(), secrets.clone());
        assert!(reloaded.is_enabled());
        assert_eq!(reloaded.authenticate(&token).unwrap().client_id, client.id);
        assert!(reloaded.view().unwrap().clients[0].last_used_at.is_some());
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_access_starts_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ACCESS_FILE), b"{not json").unwrap();
        let access = AgentAccess::load(dir.path(), Arc::new(MemorySecrets::default()));
        assert!(!access.is_enabled());
        assert!(access.view().unwrap().clients.is_empty());
        let aside = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(aside, "the unreadable file is kept for inspection");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, _, access) = setup();
        access.set_enabled(true).unwrap();
        let mode = std::fs::metadata(dir.path().join(ACCESS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_dev_profile_never_shares_a_keyring_entry_with_the_real_app() {
        assert_eq!(
            keyring_target(None, "cl_1"),
            (KEYRING_SERVICE.to_string(), "cl_1".to_string())
        );
        let (service, account) = keyring_target(Some("agents"), "cl_1");
        assert_ne!(service, KEYRING_SERVICE);
        assert_eq!(account, "cl_1@dev-profile:agents");
    }
}
