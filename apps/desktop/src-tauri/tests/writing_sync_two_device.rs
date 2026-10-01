//! WS6 two-device acceptance for the writing sync (writing_envelopes v1).
//!
//! This suite exercises the REAL desktop writing cycle end to end: the public
//! `writing::repository` save path (transactional capture into the writing
//! outbox), `sync::engine::run_cycle` (which runs the bounded W-ENGINE writing
//! phase), and the real `sync::http::HttpSyncApi` transport — two independent
//! devices, each with its OWN temp `app_data_dir` and its OWN SQLite DB built
//! from the checked-in `tests/fixtures/schema_full.sql` application schema.
//! There is no mock `SyncApi` here and no in-process server: the client does
//! not depend on the EntropIA-Cloud server library, so the only dependency-free
//! seam for a real loopback server is spawning the Cloud server binary itself
//! (exactly what `scripts/e2e-local.ps1` does in the Cloud repo).
//!
//! Server resolution, in order:
//!   1. `ENTROPIA_SYNC_E2E_SERVER` — a server already running (e2e script).
//!   2. `ENTROPIA_SYNC_SERVER_BIN` — explicit path to `entropia-sync-server`.
//!   3. The sibling checkout `EntropIA-Cloud/target/{debug,release}` (newest
//!      binary wins), spawned on an ephemeral loopback port with a throwaway
//!      `SYNC_DATA_DIR` and `SYNC_REGISTRATION_OPEN=true`.
//!
//! With none available the tests print a skip notice and return — the same
//! loud-skip contract as `tests/sync_e2e.rs`. A reachable server that does not
//! advertise `writing-envelope-v1` fails loudly (a stale binary cannot be a
//! WS6 acceptance target).
//!
//! Scenarios (WS6 acceptance):
//!   * A pushes a created document; B receives its text/title/status.
//!   * Divergent edits produce ONE visible conflict copy, on both devices.
//!   * Image blobs upload before the envelope travels, and a pulled image is
//!     hash-verified BEFORE the envelope is applied (fault-injected).
//!   * Reopening the databases never duplicates documents or conflict copies.
//!
//! Everything runs on throwaway temp dirs; no production database is touched.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use entropia_desktop_lib::sync::capture::ensure_capture;
use entropia_desktop_lib::sync::engine::{read_schema_tag, run_cycle};
use entropia_desktop_lib::sync::http::{HttpSyncApi, LoginRequest, RegisterRequest, SyncApi};
use entropia_desktop_lib::sync::session::meta_set;
use entropia_desktop_lib::writing::repository::{
    create_document, list_documents, load_document, save_document, set_status, NewDocument,
    SaveDocument,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The full application schema fixture (the same file the in-tree unit tests
/// load via `test_support::SCHEMA_FIXTURE`). Included directly because that
/// helper is `#[cfg(test)]` inside the lib and unreachable from an external
/// integration crate (see `tests/sync_e2e.rs`).
const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");

/// Base URL of an already-running server (the `scripts/e2e-local.ps1` seam).
const SERVER_ENV: &str = "ENTROPIA_SYNC_E2E_SERVER";
/// Explicit path to a built `entropia-sync-server` binary.
const SERVER_BIN_ENV: &str = "ENTROPIA_SYNC_SERVER_BIN";
/// A password that satisfies the server's ≥10-char rule (PROTOCOL auth).
const PASSWORD: &str = "ws6-password-123";

static EMAIL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique account email per test run: `ws6-{pid}-{counter}@entropia.test`.
fn fresh_email() -> String {
    let pid = std::process::id();
    let n = EMAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("ws6-{pid}-{n}@entropia.test")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// A minimal PNG: magic header plus a distinctive payload. Enough for the
/// attachment scanner's media sniffing (`detect_media_type` reads the prefix).
fn png_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(payload);
    bytes
}

/// Document content shaped like the frontend editor export (`schemaVersion`
/// wrapper + ProseMirror `doc`), matching what `writing::sync_files` scans.
fn content_text(text: &str) -> String {
    serde_json::json!({
        "schemaVersion": 1,
        "doc": {
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": text }]
            }]
        }
    })
    .to_string()
}

/// The same content plus one `writingImage` node — the exact reference shape
/// the export walker collects (`writingImage` node `attrs.src`).
fn content_with_image(text: &str, rel_path: &str) -> String {
    serde_json::json!({
        "schemaVersion": 1,
        "doc": {
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": text }] },
                { "type": "writingImage", "attrs": { "src": rel_path } }
            ]
        }
    })
    .to_string()
}

/// Concatenates every `text` string inside a content JSON, so assertions can
/// speak about visible text without hardcoding the document tree shape.
fn text_of(content_json: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(content_json).expect("parse content json");
    let mut out = String::new();
    collect_text(&value, &mut out);
    out
}

fn collect_text(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, nested) in map {
                if key == "text" {
                    if let Some(fragment) = nested.as_str() {
                        if !out.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(fragment);
                    }
                }
                collect_text(nested, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, out);
            }
        }
        _ => {}
    }
}

/// Binds :0, lets the OS pick a free loopback port, releases it (the same
/// accepted TOCTOU window as `scripts/e2e-local.ps1`).
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener
        .local_addr()
        .expect("ephemeral port address")
        .port()
}

fn open_conn(db_path: &Path) -> Connection {
    let conn = Connection::open(db_path).expect("open device db");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )
    .expect("configure pragmas");
    conn
}

// ---------------------------------------------------------------------------
// Test server (real EntropIA-Cloud binary on loopback, or an external one)
// ---------------------------------------------------------------------------

struct TestServer {
    base_url: String,
    /// The throwaway `SYNC_DATA_DIR` this test owns — `None` for an external
    /// server, which also disables the store-level fault injection.
    data_dir: Option<tempfile::TempDir>,
    child: Option<Child>,
}

impl TestServer {
    async fn start() -> Option<TestServer> {
        if let Ok(url) = std::env::var(SERVER_ENV) {
            let trimmed = url.trim().trim_end_matches('/').to_string();
            if !trimmed.is_empty() {
                return Some(TestServer {
                    base_url: trimmed,
                    data_dir: None,
                    child: None,
                });
            }
        }
        let bin = find_server_bin()?;
        let data_dir = tempfile::tempdir().expect("server data dir");
        let port = free_port();
        let bind = format!("127.0.0.1:{port}");
        let child = Command::new(&bin)
            .env("SYNC_BIND_ADDR", &bind)
            .env("SYNC_DATA_DIR", data_dir.path())
            .env("SYNC_REGISTRATION_OPEN", "true")
            .env("SYNC_MIN_FREE_DISK_MB", "1")
            .env("SYNC_MAX_ACCOUNTS", "100")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn entropia-sync-server");
        let base_url = format!("http://{bind}");
        let api = HttpSyncApi::new(&base_url).expect("server base url");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if api.health().await.is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "entropia-sync-server did not answer /v1/health at {base_url} within 30s"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Some(TestServer {
            base_url,
            data_dir: Some(data_dir),
            child: Some(child),
        })
    }

    /// The server's content-addressed blob store path (PROTOCOL "Blobs":
    /// `{data_dir}/blobs/{account}/{hash[0..2]}/{hash}`). `None` for an
    /// external server, where the store is not this test's to touch.
    fn store_path(&self, account_id: &str, sha256: &str) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|dir| {
            dir.path()
                .join("blobs")
                .join(account_id)
                .join(&sha256[..2])
                .join(sha256)
        })
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Locates a server binary: explicit env override first, then the sibling
/// Cloud checkout, newest build wins (the newest binary is the one built from
/// the sources that carry the `writing-envelope-v1` capability).
fn find_server_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(SERVER_BIN_ENV) {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../EntropIA-Cloud/target");
    let names: &[&str] = if cfg!(windows) {
        &["entropia-sync-server.exe"]
    } else {
        &["entropia-sync-server"]
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for dir in ["debug", "release"] {
        for name in names {
            let candidate = root.join(dir).join(name);
            if let Ok(modified) = std::fs::metadata(&candidate).and_then(|m| m.modified()) {
                if best.as_ref().is_none_or(|(best_at, _)| modified > *best_at) {
                    best = Some((modified, candidate));
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

fn server_skip_notice() {
    eprintln!(
        "[writing-sync-two-device] SKIP: no test server available. Set {SERVER_ENV}=<base-url> to \
         a running entropia-sync-server, or {SERVER_BIN_ENV}=<path> to a built server binary (the \
         default probe looks under ../../../../EntropIA-Cloud/target)."
    );
}

// ---------------------------------------------------------------------------
// Device harness (one simulated device per temp database)
// ---------------------------------------------------------------------------

/// One simulated device: a temp app-data dir, a SQLite DB carrying the full
/// application schema + sync schema + capture triggers, the real writing
/// repository API, the real `HttpSyncApi`, and an in-memory session (the token
/// never touches the OS keyring, so two devices coexist on one machine).
struct Device {
    /// Kept alive so the temp dir outlives the test.
    _tempdir: tempfile::TempDir,
    app_data_dir: PathBuf,
    db_path: PathBuf,
    conn: Connection,
    api: HttpSyncApi,
    server_url: String,
    token: String,
    device_id: String,
    account_id: String,
}

impl Device {
    fn new(server_url: &str) -> Device {
        let tempdir = tempfile::tempdir().expect("device temp dir");
        let app_data_dir = tempdir.path().to_path_buf();
        std::fs::create_dir_all(app_data_dir.join("assets")).expect("assets dir");
        let db_path = app_data_dir.join("entropia.sqlite");

        let conn = open_conn(&db_path);
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("application schema");
        // Migration heads: a real schema tag for `read_schema_tag`, plus the
        // two writing heads the in-tree fixtures record (`0035` gates the
        // writing repository, `0036` the journal tables).
        conn.execute(
            "INSERT INTO _migrations(name, applied_at) VALUES (?1, ?2)",
            rusqlite::params!["0028_vec_assets_embedding_contract", now_ms()],
        )
        .expect("migration head");
        conn.execute_batch(
            "INSERT INTO _migrations(name, applied_at) VALUES('0035_writing_workspace', 1);
             INSERT INTO _migrations(name, applied_at) VALUES('0036_writing_journal', 2);",
        )
        .expect("writing migration heads");
        ensure_capture(&conn).expect("sync schema + capture triggers");

        let api = HttpSyncApi::new(server_url).expect("http sync api");
        Device {
            _tempdir: tempdir,
            app_data_dir,
            db_path,
            conn,
            api,
            server_url: server_url.to_string(),
            token: String::new(),
            device_id: String::new(),
            account_id: String::new(),
        }
    }

    /// Registers a brand-new account and logs this device into it. The server
    /// must run with `SYNC_REGISTRATION_OPEN=true`. Returns the account email,
    /// so a SECOND device can log into the same account.
    async fn register_and_login(&mut self) -> String {
        let email = fresh_email();
        self.api
            .register(RegisterRequest {
                email: email.clone(),
                password: PASSWORD.to_string(),
            })
            .await
            .expect("register account");
        self.login(&email).await;
        email
    }

    /// Logs this device into an existing account, persisting the session the
    /// same way `sync_login` does (minus the keyring): token in-memory, session
    /// meta in `sync_meta` — including a fresh session incarnation, which the
    /// writing phase requires before it sends or settles anything.
    async fn login(&mut self, email: &str) -> String {
        let resp = self
            .api
            .login(LoginRequest {
                email: email.to_string(),
                password: PASSWORD.to_string(),
                device_name: format!("ws6-{}", std::process::id()),
                platform: "test".to_string(),
            })
            .await
            .expect("login");
        self.token = resp.device_token;
        self.device_id = resp.device_id;
        self.account_id = resp.account_id.clone();

        meta_set(&self.conn, "server_url", &self.server_url).expect("server_url meta");
        meta_set(&self.conn, "account_id", &resp.account_id).expect("account_id meta");
        meta_set(&self.conn, "account_email", email).expect("account_email meta");
        meta_set(&self.conn, "device_id", &self.device_id).expect("device_id meta");
        meta_set(&self.conn, "capture_enabled", "1").expect("capture_enabled meta");
        meta_set(
            &self.conn,
            "sync_session_incarnation",
            &Uuid::new_v4().to_string(),
        )
        .expect("session incarnation meta");
        resp.account_id
    }

    /// Fails loudly when the server cannot be a WS6 target: a server without
    /// `writing-envelope-v1` exercises nothing of the writing transport.
    async fn assert_writing_capability(&self) {
        let schema_tag = read_schema_tag(&self.conn).expect("schema tag");
        let response = self
            .api
            .pull(&self.token, &schema_tag, 0, 1)
            .await
            .expect("capability probe pull");
        assert!(
            response.supports_writing_envelope_v1(),
            "the test server does not advertise writing-envelope-v1; rebuild the \
             EntropIA-Cloud server or point {SERVER_ENV} at a current server"
        );
    }

    /// Runs one full sync cycle (corpus + bounded writing phase) and panics
    /// with a legible message on a cycle error.
    async fn sync(&self) {
        let warn = |msg: String| eprintln!("[writing-sync-two-device warn] {msg}");
        run_cycle(
            &self.api,
            &self.token,
            &self.conn,
            &self.app_data_dir,
            &warn,
        )
        .await
        .unwrap_or_else(|e| panic!("sync cycle failed: {e:?}"));
    }

    /// Closes and reopens the device database on the same path — the "reopen"
    /// of the acceptance scenarios.
    fn reopen(&mut self) {
        self.conn = open_conn(&self.db_path);
    }

    fn documents(&self) -> Vec<entropia_desktop_lib::writing::repository::DocumentRow> {
        list_documents(&self.conn, &[]).expect("list documents")
    }

    fn document_count(&self) -> usize {
        self.documents().len()
    }

    /// Durable writing-work counters: (receive queue, outbox) — the two
    /// `sync_meta` queues the writing cycle drains.
    fn pending_counts(&self) -> (i64, i64) {
        let receives: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'writing_receive:%'",
                [],
                |row| row.get(0),
            )
            .expect("receive queue count");
        let outbox: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'writing_outbox:%'",
                [],
                |row| row.get(0),
            )
            .expect("outbox count");
        (receives, outbox)
    }
}

/// Alternates full cycles so each device can flush and then observe what its
/// peer pushed after its own last pull. Bounded: convergence is asserted by
/// the callers, never assumed from the loop.
async fn converge(a: &Device, b: &Device, rounds: usize) {
    for _ in 0..rounds {
        a.sync().await;
        b.sync().await;
    }
}

fn create_doc(conn: &Connection, title: &str, content_json: &str) -> String {
    let id = Uuid::new_v4().to_string();
    create_document(
        conn,
        NewDocument {
            id: id.clone(),
            title: title.to_string(),
            document_type: "article".to_string(),
            schema_version: 1,
            content_json: content_json.to_string(),
        },
    )
    .expect("create document");
    id
}

fn save_doc(conn: &mut Connection, id: &str, expected_revision: i64, content_json: &str) {
    save_document(
        conn,
        SaveDocument {
            document_id: id.to_string(),
            expected_revision,
            content_json: content_json.to_string(),
            schema_version: 1,
            plain_text_cache: None,
            citations: Vec::new(),
            zotero_citations: Vec::new(),
            provenance: Vec::new(),
        },
    )
    .expect("save document");
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// Two temp databases; A pushes a created document and B receives its text,
/// title and status. Reopening both databases must not duplicate anything.
#[tokio::test]
async fn two_device_create_push_receive_text_title_status() {
    let Some(server) = TestServer::start().await else {
        server_skip_notice();
        return;
    };
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_writing_capability().await;
    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    b.assert_writing_capability().await;

    // A creates and annotates one document, entirely locally.
    let id = create_doc(&a.conn, "Acta WS6", &content_text("Texto base A"));
    set_status(&a.conn, &id, "archived").expect("set status");

    converge(&a, &b, 4).await;

    // "A push creates document": A's outbox drained into the server...
    assert_eq!(
        a.pending_counts(),
        (0, 0),
        "A's create/status work must be pushed and acknowledged"
    );
    // ...and B receives the exact text, title and status.
    let received = load_document(&b.conn, &id).expect("B receives the document");
    assert_eq!(received.title, "Acta WS6", "B receives the title");
    assert_eq!(received.status, "archived", "B receives the status");
    assert!(
        text_of(&received.current_content_json).contains("Texto base A"),
        "B receives the text, got {:?}",
        text_of(&received.current_content_json)
    );

    // Reopen does not duplicate: fresh connections, more cycles, one document.
    a.reopen();
    b.reopen();
    converge(&a, &b, 3).await;
    assert_eq!(
        a.document_count(),
        1,
        "A keeps exactly one document after reopen"
    );
    assert_eq!(
        b.document_count(),
        1,
        "B keeps exactly one document after reopen"
    );
}

/// Divergent edits on the same document must converge to the LWW winner plus
/// exactly ONE visible conflict copy carrying the losing manuscript — visible
/// on BOTH devices (the copy syncs back), and never duplicated by retries or
/// reopens.
#[tokio::test]
async fn two_device_divergent_edits_preserve_one_visible_conflict_copy() {
    let Some(server) = TestServer::start().await else {
        server_skip_notice();
        return;
    };
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_writing_capability().await;
    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    b.assert_writing_capability().await;

    let id = create_doc(&a.conn, "Manuscrito", &content_text("Texto base"));
    converge(&a, &b, 4).await;
    assert_eq!(
        b.document_count(),
        1,
        "both devices share the base document"
    );

    // B edits while offline (no cycle in between)...
    let revision_b = load_document(&b.conn, &id)
        .expect("B base revision")
        .revision;
    save_doc(&mut b.conn, &id, revision_b, &content_text("Texto local B"));
    // ...then A diverges with a LATER edit (LWW winner) and pushes first.
    std::thread::sleep(Duration::from_millis(25));
    let revision_a = load_document(&a.conn, &id)
        .expect("A base revision")
        .revision;
    save_doc(
        &mut a.conn,
        &id,
        revision_a,
        &content_text("Texto remoto A"),
    );
    a.sync().await;

    converge(&a, &b, 5).await;

    // On B: the winner landed and the losing local manuscript survived as
    // exactly one visible conflict copy.
    let docs_b = b.documents();
    assert_eq!(
        docs_b.len(),
        2,
        "divergent edits must produce exactly one visible conflict copy on B \
         (receive queue {:?}, outbox {:?})",
        b.pending_counts().0,
        b.pending_counts().1
    );
    let winner = load_document(&b.conn, &id).expect("winner document");
    assert!(
        text_of(&winner.current_content_json).contains("Texto remoto A"),
        "the LWW winner lands on B, got {:?}",
        text_of(&winner.current_content_json)
    );
    let copies: Vec<_> = docs_b.iter().filter(|doc| doc.id != id).collect();
    assert_eq!(copies.len(), 1, "exactly one conflict copy on B");
    assert!(
        text_of(&copies[0].current_content_json).contains("Texto local B"),
        "the conflict copy carries the losing text, got {:?}",
        text_of(&copies[0].current_content_json)
    );

    // The copy syncs back: A sees the same two manuscripts.
    let docs_a = a.documents();
    assert_eq!(
        docs_a.len(),
        2,
        "the conflict copy is visible on A too (receive queue {:?}, outbox {:?})",
        a.pending_counts().0,
        a.pending_counts().1
    );

    // Reopen does not duplicate: still exactly one copy on each device.
    a.reopen();
    b.reopen();
    converge(&a, &b, 3).await;
    assert_eq!(
        b.document_count(),
        2,
        "no duplicate conflict copy on B after reopen"
    );
    assert_eq!(
        a.document_count(),
        2,
        "no duplicate conflict copy on A after reopen"
    );
}

/// Image blobs: the blob is uploaded BEFORE the envelope travels (fail-closed:
/// no file, no envelope), and a pulled image is hash-verified BEFORE the row is
/// applied (fault-injected corruption never applies; verified bytes do).
#[tokio::test]
async fn image_blob_upload_precedes_envelope_and_download_verifies_before_apply() {
    let Some(server) = TestServer::start().await else {
        server_skip_notice();
        return;
    };
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_writing_capability().await;
    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    b.assert_writing_capability().await;

    let img1 = png_bytes(b"ws6-image-one");
    let sha1 = sha256_hex(&img1);
    let rel1 = format!("writing-images/{sha1}.png");
    let id1 = create_doc(
        &a.conn,
        "Con imagen",
        &content_with_image("Texto con imagen", &rel1),
    );

    // Upload before envelope, fail-closed side: with the image file absent the
    // envelope must NOT travel, and the document stays pending on A.
    a.sync().await;
    converge(&a, &b, 2).await;
    assert!(
        load_document(&b.conn, &id1).is_err(),
        "no envelope may reach B while its blob is unprovable"
    );
    assert_eq!(
        a.pending_counts().1,
        1,
        "the document keeps its outbox entry until its blob is provable"
    );

    // With the image in place the blob uploads and the envelope follows.
    write_image(&a, &sha1, &img1);
    a.sync().await;
    assert_eq!(
        a.pending_counts().1,
        0,
        "once the blob uploads, the envelope pushes and is acknowledged"
    );
    if let Some(store) = server.store_path(&a.account_id, &sha1) {
        let stored = std::fs::read(&store).expect("blob stored on the server");
        assert_eq!(
            sha256_hex(&stored),
            sha1,
            "the server holds verified blob bytes alongside the envelope"
        );
    }

    // B applies the row only after a verified download installed the image.
    converge(&a, &b, 3).await;
    let received = load_document(&b.conn, &id1).expect("B applies the imaged document");
    assert!(
        text_of(&received.current_content_json).contains("Texto con imagen"),
        "B receives the imaged text"
    );
    let installed = b.app_data_dir.join(&rel1);
    let installed_bytes = std::fs::read(&installed).expect("image installed on B");
    assert_eq!(
        sha256_hex(&installed_bytes),
        sha1,
        "B's installed image is byte-verified"
    );

    // Fault injection: a second image whose stored blob is corrupted after
    // upload. B must NOT apply the row and must NOT publish unverified bytes.
    let img2 = png_bytes(b"ws6-image-two");
    let sha2 = sha256_hex(&img2);
    let rel2 = format!("writing-images/{sha2}.png");
    let id2 = create_doc(
        &a.conn,
        "Segunda imagen",
        &content_with_image("Texto imagen dos", &rel2),
    );
    write_image(&a, &sha2, &img2);
    a.sync().await;

    let mut corrupted_store = None;
    if let Some(store) = server.store_path(&a.account_id, &sha2) {
        assert!(store.exists(), "second blob uploaded before its envelope");
        let mut bad = img2.clone();
        bad[16] ^= 0xff;
        std::fs::write(&store, &bad).expect("corrupt stored blob");
        corrupted_store = Some(store);
    }

    if let Some(store) = corrupted_store {
        converge(&a, &b, 3).await;
        assert!(
            load_document(&b.conn, &id2).is_err(),
            "a blob that fails hash verification must never be applied"
        );
        assert!(
            !b.app_data_dir.join(&rel2).exists(),
            "no unverified bytes are published into B's app data"
        );

        // Restoring the verified bytes unblocks the retained receive.
        std::fs::write(&store, &img2).expect("restore stored blob");
        converge(&a, &b, 3).await;
    }

    let received2 = load_document(&b.conn, &id2).expect("B applies after the blob verifies");
    assert!(
        text_of(&received2.current_content_json).contains("Texto imagen dos"),
        "B receives the second imaged text"
    );
    let installed2 = b.app_data_dir.join(&rel2);
    let installed2_bytes = std::fs::read(&installed2).expect("second image installed on B");
    assert_eq!(
        sha256_hex(&installed2_bytes),
        sha2,
        "B's second image is byte-verified"
    );

    // Reopen does not duplicate either imaged document.
    a.reopen();
    b.reopen();
    converge(&a, &b, 3).await;
    assert_eq!(
        a.document_count(),
        2,
        "no duplicate imaged documents on A after reopen"
    );
    assert_eq!(
        b.document_count(),
        2,
        "no duplicate imaged documents on B after reopen"
    );
}

fn write_image(device: &Device, sha256: &str, bytes: &[u8]) {
    let dir = device.app_data_dir.join("writing-images");
    std::fs::create_dir_all(&dir).expect("writing-images dir");
    std::fs::write(dir.join(format!("{sha256}.png")), bytes).expect("write image file");
}
