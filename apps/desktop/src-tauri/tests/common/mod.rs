//! Shared fixtures for the sync integration suites in this crate.
//!
//! `MockServer` is an in-memory [`SyncApi`] double that behaves like a small
//! EntropIA Cloud server for the parts the derived-cache suite needs: capability
//! advertisement on ordinary responses, two independent row channels (corpus and
//! `bibliography-derived-v1`) with their own sequence cursors and per-account
//! scoping, idempotent inserts (`ON CONFLICT DO NOTHING` semantics) and a health
//! response with a configurable `max_push_bytes`.
//!
//! `TestDevice` is one simulated device: a temp `app_data_dir`, its own SQLite
//! file carrying the real application schema (`tests/fixtures/schema_full.sql`),
//! the sync schema and its session metadata. It never touches the OS keyring and
//! never opens a socket.
//!
//! The session metadata is written here exactly as the crate-private
//! `sync::session::write_sync_session` does (account, URL, device, incarnation,
//! both durable derived-cache marks); the code under test — the engine cycle and
//! the derived transport — is entirely production code.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use entropia_desktop_lib::sync::capture::ensure_capture;
use entropia_desktop_lib::sync::engine::run_cycle;
use entropia_desktop_lib::sync::http::{
    BlobExists, DeleteAccountRequest, DevicesResponse, HealthLimits, HealthResponse, LoginRequest,
    LoginResponse, NotificationItem, PlanCatalogItem, PlanChangeRequestResponse, PullResponse,
    PullRow, PushChange, PushRequest, PushResponse, PushResult, RegisterRequest, RegisterResponse,
    SyncApi, SyncError, UsageResponse,
};
use entropia_desktop_lib::sync::session::{clear_sync_state, meta_set};
use rusqlite::Connection;
use serde_json::{json, Value};

pub const SCHEMA_FIXTURE: &str = include_str!("../fixtures/schema_full.sql");

/// Capability tokens this suite negotiates.
pub const DERIVED_CAPABILITY: &str = "bibliography-derived-v1";
pub const WEB_CAPTURE_CAPABILITY: &str = "web-capture-v1";

/// The `sync_meta` key the session writer owns; it is crate-private, so the
/// fixture writes the same literal (`sync/session.rs`).
const SESSION_INCARNATION_KEY: &str = "sync_session_incarnation";

// ---------------------------------------------------------------------------
// In-memory server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StoredRow {
    seq: i64,
    account_id: String,
    table: String,
    row_id: String,
    payload: Value,
    deleted: bool,
}

#[derive(Default)]
struct Channel {
    rows: Vec<StoredRow>,
    next_seq: i64,
}

impl Channel {
    fn insert(&mut self, account_id: &str, change: &PushChange) -> i64 {
        let existing = self.rows.iter().find(|row| {
            row.account_id == account_id && row.table == change.table && row.row_id == change.row_id
        });
        if let Some(row) = existing {
            // `ON CONFLICT DO NOTHING`: first writer wins, the sequence stands.
            return row.seq;
        }
        self.next_seq += 1;
        let seq = self.next_seq;
        self.rows.push(StoredRow {
            seq,
            account_id: account_id.to_string(),
            table: change.table.clone(),
            row_id: change.row_id.clone(),
            payload: change.payload.clone().unwrap_or(Value::Null),
            deleted: change.op == "delete",
        });
        seq
    }

    fn page(
        &self,
        account_id: &str,
        derived: bool,
        since: i64,
        limit: i64,
    ) -> (Vec<PullRow>, i64, bool) {
        let mut rows: Vec<&StoredRow> = self
            .rows
            .iter()
            .filter(|row| row.account_id == account_id && row.seq > since)
            .filter(|row| is_derived_table(&row.table) == derived)
            .collect();
        rows.sort_by_key(|row| row.seq);
        let limit = limit.max(1) as usize;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_since = rows.last().map(|row| row.seq).unwrap_or(since);
        (
            rows.into_iter()
                .map(|row| PullRow {
                    table: row.table.clone(),
                    row_id: row.row_id.clone(),
                    server_seq: row.seq,
                    deleted: row.deleted,
                    changed_at: 0,
                    device_id: "server".to_string(),
                    payload: if row.deleted {
                        None
                    } else {
                        Some(row.payload.clone())
                    },
                })
                .collect(),
            next_since,
            has_more,
        )
    }
}

fn is_derived_table(table: &str) -> bool {
    matches!(
        table,
        "derived_vectors" | "derived_ocr_pages" | "derived_contracts"
    )
}

/// An in-memory server with the two channels, capability advertising, epoch and
/// quota knobs. Every derived request is recorded so a test can prove that a
/// legacy server received none, that batches respect the byte budget, or that a
/// row of another account never travelled.
pub struct MockServer {
    epoch: Mutex<String>,
    capabilities: Mutex<Vec<String>>,
    pub max_push_bytes: i64,
    tokens: Mutex<HashMap<String, String>>,
    channels: Mutex<(Channel, Channel)>,
    pub derived_pushes: Mutex<Vec<PushRequest>>,
    pub ordinary_pushes: Mutex<Vec<PushRequest>>,
    pub derived_pull_since: Mutex<Vec<i64>>,
    pub ordinary_pull_since: Mutex<Vec<i64>>,
}

impl Default for MockServer {
    fn default() -> Self {
        Self::new()
    }
}

impl MockServer {
    pub fn new() -> Self {
        Self {
            epoch: Mutex::new("epoch-1".to_string()),
            capabilities: Mutex::new(vec![
                WEB_CAPTURE_CAPABILITY.to_string(),
                DERIVED_CAPABILITY.to_string(),
            ]),
            max_push_bytes: 8 * 1024 * 1024,
            tokens: Mutex::new(HashMap::new()),
            channels: Mutex::new((Channel::default(), Channel::default())),
            derived_pushes: Mutex::new(Vec::new()),
            ordinary_pushes: Mutex::new(Vec::new()),
            derived_pull_since: Mutex::new(Vec::new()),
            ordinary_pull_since: Mutex::new(Vec::new()),
        }
    }

    /// A server of the previous generation: it never advertises the derived
    /// cache token.
    pub fn legacy() -> Self {
        let server = Self::new();
        *server.capabilities.lock().unwrap() = vec![WEB_CAPTURE_CAPABILITY.to_string()];
        server
    }

    pub fn set_epoch(&self, epoch: &str) {
        *self.epoch.lock().unwrap() = epoch.to_string();
    }

    pub fn epoch(&self) -> String {
        self.epoch.lock().unwrap().clone()
    }

    fn capabilities(&self) -> Vec<String> {
        self.capabilities.lock().unwrap().clone()
    }

    fn account_of(&self, token: &str) -> String {
        self.tokens
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .unwrap_or_default()
    }

    /// Registers the account a device token belongs to (the production server
    /// derives it from the bearer token; the fixture states it directly).
    pub fn bind_token(&self, token: &str, account_id: &str) {
        self.tokens
            .lock()
            .unwrap()
            .insert(token.to_string(), account_id.to_string());
    }

    /// Writes one row straight into the derived channel, as another device's
    /// push would have. Used to inject rows a client must validate on receipt.
    pub fn inject_derived_row(&self, account_id: &str, table: &str, row_id: &str, payload: Value) {
        let change = PushChange {
            table: table.to_string(),
            row_id: row_id.to_string(),
            op: "upsert".to_string(),
            changed_at: 0,
            base_seq: 0,
            payload: Some(payload),
        };
        let mut channels = self.channels.lock().unwrap();
        channels.1.insert(account_id, &change);
    }

    /// Every derived row the server holds for one account, as `(table, row_id)`.
    pub fn derived_keys(&self, account_id: &str) -> Vec<(String, String)> {
        let channels = self.channels.lock().unwrap();
        channels
            .1
            .rows
            .iter()
            .filter(|row| row.account_id == account_id)
            .map(|row| (row.table.clone(), row.row_id.clone()))
            .collect()
    }

    fn apply_push(&self, derived: bool, token: &str, req: PushRequest) -> PushResponse {
        let account_id = self.account_of(token);
        let mut results = Vec::with_capacity(req.changes.len());
        let mut max_server_seq = 0;
        {
            let mut channels = self.channels.lock().unwrap();
            let channel = if derived {
                &mut channels.1
            } else {
                &mut channels.0
            };
            for change in &req.changes {
                let seq = channel.insert(&account_id, change);
                max_server_seq = max_server_seq.max(seq);
                results.push(PushResult {
                    table: change.table.clone(),
                    row_id: change.row_id.clone(),
                    status: "applied".to_string(),
                    server_seq: seq,
                    winner: None,
                });
            }
        }
        PushResponse {
            results,
            max_server_seq,
            server_epoch: self.epoch(),
            server_now_ms: 1_700_000_000_000,
            capabilities: self.capabilities(),
        }
    }

    fn serve_pull(&self, derived: bool, token: &str, since: i64, limit: i64) -> PullResponse {
        let account_id = self.account_of(token);
        let (rows, next_since, has_more) = {
            let channels = self.channels.lock().unwrap();
            if derived {
                channels.1.page(&account_id, true, since, limit)
            } else {
                channels.0.page(&account_id, false, since, limit)
            }
        };
        PullResponse {
            rows,
            next_since,
            has_more,
            schema_tag: String::new(),
            server_epoch: self.epoch(),
            server_now_ms: 1_700_000_000_000,
            capabilities: self.capabilities(),
        }
    }
}

impl SyncApi for MockServer {
    async fn register(&self, _req: RegisterRequest) -> Result<RegisterResponse, SyncError> {
        Ok(RegisterResponse {
            account_id: "mock-account".to_string(),
        })
    }

    async fn login(&self, _req: LoginRequest) -> Result<LoginResponse, SyncError> {
        Ok(LoginResponse {
            account_id: "mock-account".to_string(),
            device_id: "mock-device".to_string(),
            device_token: "mock-token".to_string(),
        })
    }

    async fn logout(&self, _token: &str) -> Result<(), SyncError> {
        Ok(())
    }

    async fn devices(&self, _token: &str) -> Result<DevicesResponse, SyncError> {
        Ok(DevicesResponse {
            devices: Vec::new(),
        })
    }

    async fn revoke(&self, _token: &str, _device_id: &str) -> Result<(), SyncError> {
        Ok(())
    }

    async fn delete_account(
        &self,
        _token: &str,
        _req: DeleteAccountRequest,
    ) -> Result<(), SyncError> {
        Ok(())
    }

    async fn usage(&self, _token: &str) -> Result<UsageResponse, SyncError> {
        Ok(UsageResponse {
            rows: 0,
            blobs_count: 0,
            blobs_bytes: 0,
            quota_bytes: 0,
            plan_name: None,
            expires_at: None,
            unread_notifications: 0,
            pending_plan_request: None,
        })
    }

    async fn list_plans(&self, _token: &str) -> Result<Vec<PlanCatalogItem>, SyncError> {
        Ok(Vec::new())
    }

    async fn request_plan_change(
        &self,
        _token: &str,
        _requested_plan_id: &str,
        _note: Option<&str>,
    ) -> Result<PlanChangeRequestResponse, SyncError> {
        Err(SyncError::Api {
            status: 501,
            code: "not_implemented".to_string(),
            message: "the mock has no plan catalog".to_string(),
        })
    }

    async fn list_notifications(
        &self,
        _token: &str,
        _since: Option<&str>,
        _limit: Option<i64>,
    ) -> Result<Vec<NotificationItem>, SyncError> {
        Ok(Vec::new())
    }

    async fn mark_notification_read(&self, _token: &str, _id: &str) -> Result<(), SyncError> {
        Ok(())
    }

    async fn delete_notification(&self, _token: &str, _id: &str) -> Result<(), SyncError> {
        Ok(())
    }

    async fn health(&self) -> Result<HealthResponse, SyncError> {
        Ok(HealthResponse {
            status: "ok".to_string(),
            version: "mock".to_string(),
            epoch: self.epoch(),
            server_now_ms: 1_700_000_000_000,
            limits: HealthLimits {
                max_push_bytes: self.max_push_bytes,
                max_blob_mb: 0,
            },
        })
    }

    async fn push(
        &self,
        token: &str,
        _schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.ordinary_pushes.lock().unwrap().push(req.clone());
        Ok(self.apply_push(false, token, req))
    }

    async fn pull(
        &self,
        token: &str,
        _schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.ordinary_pull_since.lock().unwrap().push(since);
        Ok(self.serve_pull(false, token, since, limit))
    }

    async fn push_with_bibliography_derived_v1(
        &self,
        token: &str,
        _schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.derived_pushes.lock().unwrap().push(req.clone());
        Ok(self.apply_push(true, token, req))
    }

    async fn pull_with_bibliography_derived_v1(
        &self,
        token: &str,
        _schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.derived_pull_since.lock().unwrap().push(since);
        Ok(self.serve_pull(true, token, since, limit))
    }

    async fn blob_head(&self, _token: &str, _sha256: &str) -> Result<BlobExists, SyncError> {
        Ok(false)
    }

    async fn blob_put(
        &self,
        _token: &str,
        _sha256: &str,
        _bytes: Vec<u8>,
    ) -> Result<(), SyncError> {
        Ok(())
    }

    async fn blob_get(&self, _token: &str, _sha256: &str) -> Result<reqwest::Response, SyncError> {
        Err(SyncError::Api {
            status: 404,
            code: "not_found".to_string(),
            message: "the mock holds no blobs".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Device harness
// ---------------------------------------------------------------------------

/// One simulated device with its own archive file and session.
pub struct TestDevice {
    _tempdir: tempfile::TempDir,
    pub app_data_dir: PathBuf,
    pub conn: Connection,
    pub token: String,
    pub device_id: String,
    pub account_id: String,
}

impl TestDevice {
    /// A fresh device: temp `app_data_dir`, the real schema fixture, the sync
    /// schema and capture triggers. No session yet.
    pub fn new(device_id: &str, token: &str) -> Self {
        let tempdir = tempfile::tempdir().expect("temp app_data_dir");
        let app_data_dir = tempdir.path().to_path_buf();
        std::fs::create_dir_all(app_data_dir.join("assets")).expect("assets dir");
        let conn = Connection::open(app_data_dir.join("entropia.sqlite")).expect("open device db");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )
        .expect("pragmas");
        conn.execute_batch(SCHEMA_FIXTURE).expect("schema fixture");
        conn.execute(
            "INSERT INTO _migrations(name, applied_at) VALUES ('0028_vec_assets_embedding_contract', 1)",
            [],
        )
        .expect("migration head");
        ensure_capture(&conn).expect("ensure sync schema + triggers");
        Self {
            _tempdir: tempdir,
            app_data_dir,
            conn,
            token: token.to_string(),
            device_id: device_id.to_string(),
            account_id: String::new(),
        }
    }

    /// Logs this device in, writing the same session metadata the production
    /// login writes (including both durable derived-cache marks, §2.9).
    pub fn login(&mut self, server: &MockServer, server_url: &str, account_id: &str) {
        meta_set(&self.conn, "server_url", server_url).expect("server url");
        meta_set(&self.conn, "account_id", account_id).expect("account id");
        meta_set(&self.conn, "account_email", "reader@example.test").expect("email");
        meta_set(&self.conn, "device_id", &self.device_id).expect("device id");
        meta_set(&self.conn, "capture_enabled", "1").expect("capture");
        meta_set(
            &self.conn,
            SESSION_INCARNATION_KEY,
            &uuid::Uuid::new_v4().to_string(),
        )
        .expect("session incarnation");
        // The first-ever binding seals both marks (the production path does this
        // inside `write_sync_session`); later logins keep them.
        for key in ["derived_bound_account", "derived_owner_account"] {
            let present: Option<String> = self
                .conn
                .query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .ok();
            if present.is_none() {
                meta_set(&self.conn, key, account_id).expect("derived mark");
            }
        }
        self.account_id = account_id.to_string();
        server.bind_token(&self.token, account_id);
    }

    /// Logout / account change, exactly as the production command does.
    pub fn logout(&self) {
        clear_sync_state(&self.conn).expect("clear sync state");
    }

    /// Runs one full sync cycle against the mock.
    pub async fn sync(&self, server: &MockServer) {
        let warn = |message: String| eprintln!("[derived-e2e warn] {message}");
        run_cycle(server, &self.token, &self.conn, &self.app_data_dir, &warn)
            .await
            .expect("sync cycle");
    }

    /// Seeds one paid vector row exactly as the publish transaction does
    /// (`origin='local'`, `pushed=0`, the session account) and mirrors its
    /// immutable contract row. The writers themselves are covered by the
    /// bibliography suites; this fixture is about the transport.
    pub fn seed_paid_vector(
        &self,
        contract_hash: &str,
        input_hash: &str,
        dimensions: i64,
        value: u8,
    ) {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO bibliographic_embedding_contracts
                   (contract_hash, provider, model, dimensions, chunking_contract, created_at)
                 VALUES (?1, 'openrouter', 'text-embedding-3-large', ?2, 'chunks-v1', 1)",
                rusqlite::params![contract_hash, dimensions],
            )
            .expect("source contract");
        self.conn
            .execute(
                "INSERT OR IGNORE INTO sync_derived_contracts
                   (contract_hash, provider, model, dimensions, chunking_contract)
                 VALUES (?1, 'openrouter', 'text-embedding-3-large', ?2, 'chunks-v1')",
                rusqlite::params![contract_hash, dimensions],
            )
            .expect("mirrored contract");
        let embedding = vec![value; (dimensions * 4) as usize];
        self.conn
            .execute(
                "INSERT OR IGNORE INTO sync_derived_vectors
                   (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'local', 0, 1)",
                rusqlite::params![
                    self.account_id,
                    contract_hash,
                    input_hash,
                    dimensions,
                    embedding
                ],
            )
            .expect("paid vector");
    }

    /// Seeds one paid OCR answer as the publish transaction does.
    pub fn seed_paid_ocr_page(
        &self,
        source_hash: &str,
        page_number: i64,
        ocr_contract: &str,
        outcome: &str,
        text: &str,
    ) {
        let text_hash = sha256_hex(text.as_bytes());
        self.conn
            .execute(
                "INSERT OR IGNORE INTO sync_derived_ocr_pages
                   (account_id, source_hash, page_number, ocr_contract, outcome, text_content,
                    text_hash, origin, pushed, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'local', 0, 1)",
                rusqlite::params![
                    self.account_id,
                    source_hash,
                    page_number,
                    ocr_contract,
                    outcome,
                    text,
                    text_hash
                ],
            )
            .expect("paid OCR page");
    }

    /// One corpus row (`collections`), captured by the real triggers.
    pub fn seed_collection(&self, id: &str) {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO collections(id, name, created_at, updated_at)
                 VALUES (?1, ?2, 1, 1)",
                rusqlite::params![id, format!("Collection {id}")],
            )
            .expect("collection row");
    }

    pub fn pushed_vectors(&self, account_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sync_derived_vectors WHERE account_id = ?1 AND pushed = 1",
                [account_id],
                |row| row.get(0),
            )
            .expect("count pushed vectors")
    }

    pub fn pending_rows(&self, account_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM sync_derived_vectors
                          WHERE account_id = ?1 AND pushed = 0)
                      + (SELECT COUNT(*) FROM sync_derived_ocr_pages
                          WHERE account_id = ?1 AND pushed = 0)",
                [account_id],
                |row| row.get(0),
            )
            .expect("count pending rows")
    }

    pub fn scalar(&self, sql: &str) -> i64 {
        self.conn
            .query_row(sql, [], |row| row.get(0))
            .expect("scalar")
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .ok()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A minimal valid derived vector payload (base64 of `dimensions` LE f32).
pub fn vector_payload(contract_hash: &str, input_hash: &str, dimensions: i64, value: f32) -> Value {
    use base64::Engine as _;
    let mut bytes = Vec::with_capacity((dimensions as usize) * 4);
    for _ in 0..dimensions {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    json!({
        "contract_hash": contract_hash,
        "input_hash": input_hash,
        "dimensions": dimensions,
        "embedding": base64::engine::general_purpose::STANDARD.encode(bytes),
        "created_at": 1,
    })
}
