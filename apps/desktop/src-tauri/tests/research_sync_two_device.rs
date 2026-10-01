//! IS6 two-device acceptance for terminal Investigations sync
//! (`research_envelopes` v1).
//!
//! This suite exercises the REAL desktop Investigations sync end to end: two
//! independent devices, each with its OWN temp `app_data_dir`, its OWN app
//! SQLite DB built from the checked-in `tests/fixtures/schema_full.sql`
//! application schema, and its OWN `research/estado.sqlite` created with
//! `entropia_agent::estado::EstadoDb::abrir`. Every cycle is the public
//! `sync::engine::run_cycle` — which runs the bounded research phase
//! (`research_cycle`) — over the real `sync::http::HttpSyncApi` transport
//! against a real EntropIA-Cloud server binary on loopback. There is no mock
//! `SyncApi` here and no in-process server: exactly like
//! `tests/writing_sync_two_device.rs`, the only dependency-free seam for a real
//! loopback server is spawning the Cloud server binary itself.
//!
//! Server resolution, in order:
//!   1. `ENTROPIA_SYNC_E2E_SERVER` — a server already running (e2e script).
//!   2. `ENTROPIA_SYNC_SERVER_BIN` — explicit path to `entropia-sync-server`.
//!   3. The sibling checkout `EntropIA-Cloud/target` (plain and
//!      `research-verify` build dirs, newest binary wins), spawned on an
//!      ephemeral loopback port with a throwaway `SYNC_DATA_DIR` and
//!      `SYNC_REGISTRATION_OPEN=true`.
//! With none available the tests print a loud skip notice and return — the
//! same loud-skip contract as `tests/sync_e2e.rs` and
//! `tests/writing_sync_two_device.rs`. A reachable server that does not
//! advertise `research-envelope-v1` FAILS loudly: a stale/legacy server can
//! never be a research E2E acceptance target, so the capability probe asserts
//! the exact current token before any scenario runs.
//!
//! Test-only fixture SQL policy: the production terminal capture hooks
//! (`research_capture::enqueue_terminal_job` and the delete-intent lifecycle)
//! are `pub(crate)` and unreachable from an integration crate. They are
//! separately covered by the in-tree lib tests
//! (`sync/research_capture_tests.rs`, `sync/research_cycle_tests.rs`). This
//! acceptance suite therefore writes the exact durable outbox shape directly
//! (`sync_meta` `research_outbox:<job_id>` entries) only where the test must
//! own `changed_at` — the LWW adjudication input — or enqueue a `D` tombstone
//! for a locally deleted job. Scenario 1 needs no fixture SQL at all: the
//! production `seed_terminal_outbox` backfill queues the created job.
//!
//! Scenarios (IS6 acceptance):
//!   * A creates a terminal job (fixture `modo='research'`, `status='done'`,
//!     valid `close_reason`, request/report/archive artifacts and a real
//!     `research/artifacts/<job>/report.md`) → real Cloud push with the report
//!     blob → B discovers the capability, catches up, downloads/verifies the
//!     blob and projects the readable estado aggregate. Repeats and reopens
//!     never duplicate and never fabricate active/billable rows.
//!   * Divergent terminal report edits queued on BOTH devices before any push,
//!     `changed_at` arranged so the first push wins the LWW and the second is
//!     adjudicated `lww_lost` (observable through the lww-lost-only warning).
//!     The losing device preserves exactly one `sync_conflicts` entry for
//!     `research_envelopes`, the winner projection is readable, and repeat
//!     cycles never duplicate the conflict, the job or its artifacts.
//!   * One terminal job deleted locally on A plus a `D` outbox tombstone → the
//!     peer deletes its projection (and report.md) and repeated/reopened cycles
//!     never resurrect it. Missing-source manifest entries stay provenance-only
//!     (they reach the archive artifact and never fabricate `sources`/`evidence`
//!     rows).
//!
//! Scenario 2 note on the report install: `research_blobs` publishes `report.md`
//! with a no-clobber install, so a divergent local render blocks the winner's
//! report install and the receive is retained (`ExistingTargetDifferent`) —
//! which is exactly what leaves the second push a stale `base_seq` for the
//! server LWW to adjudicate. The retained winner settles only once the user
//! discards the losing local render; the test performs exactly that discard
//! between cycles (it is a local file in the device's own temp dir).
//!
//! Everything runs on throwaway temp dirs; no production database is touched
//! and the session token never touches the OS keyring.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use entropia_agent::estado::EstadoDb;
use entropia_desktop_lib::sync::capture::ensure_capture;
use entropia_desktop_lib::sync::engine::{read_schema_tag, run_cycle};
use entropia_desktop_lib::sync::http::{HttpSyncApi, LoginRequest, RegisterRequest, SyncApi};
use entropia_desktop_lib::sync::session::meta_set;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The full application schema fixture (the same file the in-tree unit tests
/// load via `test_support::SCHEMA_FIXTURE`). Included directly because that
/// helper is `#[cfg(test)]` inside the lib and unreachable from an external
/// integration crate (see `tests/writing_sync_two_device.rs`).
const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");

/// Base URL of an already-running server (the `scripts/e2e-local.ps1` seam).
const SERVER_ENV: &str = "ENTROPIA_SYNC_E2E_SERVER";
/// Explicit path to a built `entropia-sync-server` binary.
const SERVER_BIN_ENV: &str = "ENTROPIA_SYNC_SERVER_BIN";
/// A password that satisfies the server's ≥10-char rule (PROTOCOL auth).
const PASSWORD: &str = "is6-password-123";

/// The durable per-job research outbox prefix owned by
/// `sync::research_capture` (`pub(crate)`; the test writes the exact stored
/// shape directly — see the module docs).
const OUTBOX_PREFIX: &str = "research_outbox:";
/// The durable research receive-staging prefix, counted as leftover work.
const RECEIVE_PREFIX: &str = "research_receive:";

static EMAIL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique account email per test run: `is6-{pid}-{counter}@entropia.test`.
fn fresh_email() -> String {
    let pid = std::process::id();
    let n = EMAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("is6-{pid}-{n}@entropia.test")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `estado::ahora` semantics: epoch SECONDS for job timestamps.
fn now_secs() -> i64 {
    now_ms() / 1000
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

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).expect("count")
}

// ---------------------------------------------------------------------------
// Test server (real EntropIA-Cloud binary on loopback, or an external one)
// ---------------------------------------------------------------------------

struct TestServer {
    base_url: String,
    /// The throwaway `SYNC_DATA_DIR` this test owns — `None` for an external
    /// server, where the store is not this test's to touch.
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
/// Cloud checkout (plain `target/{debug,release}` and the isolated
/// `target/research-verify` build dir), newest build wins — the newest binary
/// is the one built from the sources that carry the `research-envelope-v1`
/// capability.
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
    for dir in [
        "debug",
        "release",
        "research-verify/debug",
        "research-verify/release",
    ] {
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
        "[research-sync-two-device] SKIP: no test server available. Set {SERVER_ENV}=<base-url> to \
         a running entropia-sync-server, or {SERVER_BIN_ENV}=<path> to a built server binary (the \
         default probe looks under ../../../../EntropIA-Cloud/target)."
    );
}

// ---------------------------------------------------------------------------
// Device harness (one simulated device per temp database)
// ---------------------------------------------------------------------------

/// One simulated device: a temp app-data dir, a SQLite DB carrying the full
/// application schema + sync schema + capture triggers, a separate
/// `research/estado.sqlite` created with `EstadoDb::abrir`, the real
/// `HttpSyncApi`, and an in-memory session (the token never touches the OS
/// keyring, so two devices coexist on one machine).
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
            params!["0028_vec_assets_embedding_contract", now_ms()],
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
    /// research phase requires before it sends or settles anything.
    async fn login(&mut self, email: &str) -> String {
        let resp = self
            .api
            .login(LoginRequest {
                email: email.to_string(),
                password: PASSWORD.to_string(),
                device_name: format!("is6-{}", std::process::id()),
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

    /// Fails loudly when the server cannot be an IS6 target: a server without
    /// `research-envelope-v1` exercises nothing of the research transport, and
    /// a stale binary must never silently "pass" a research E2E.
    async fn assert_research_capability(&self) {
        let schema_tag = read_schema_tag(&self.conn).expect("schema tag");
        let response = self
            .api
            .pull(&self.token, &schema_tag, 0, 1)
            .await
            .expect("capability probe pull");
        assert!(
            response.supports_research_envelope_v1(),
            "the test server does not advertise research-envelope-v1; rebuild the \
             EntropIA-Cloud server or point {SERVER_ENV} at a current server"
        );
    }

    /// Runs one full sync cycle (corpus + bounded writing + bounded
    /// Investigations research phases) with a no-op logging sink.
    async fn sync(&self) {
        let quiet = |_message: String| {};
        self.sync_with(&quiet).await;
    }

    /// Same cycle with captured warnings (the research phase reports every
    /// retained/deferred/pending decision through this sink).
    async fn sync_capturing(&self, warnings: &Mutex<Vec<String>>) {
        let sink = |message: String| {
            warnings.lock().expect("warnings").push(message);
        };
        self.sync_with(&sink).await;
    }

    async fn sync_with(&self, warn: &(dyn Fn(String) + Sync)) {
        run_cycle(&self.api, &self.token, &self.conn, &self.app_data_dir, warn)
            .await
            .unwrap_or_else(|e| panic!("sync cycle failed: {e:?}"));
    }

    /// Closes and reopens the device database on the same path — the "reopen"
    /// of the acceptance scenarios.
    fn reopen(&mut self) {
        self.conn = open_conn(&self.db_path);
    }

    // -- research state (research/estado.sqlite) ---------------------------

    fn research_state_path(&self) -> PathBuf {
        self.app_data_dir.join("research").join("estado.sqlite")
    }

    fn artifacts_root(&self) -> PathBuf {
        self.app_data_dir.join("research").join("artifacts")
    }

    fn report_path(&self, job_id: &str) -> PathBuf {
        self.artifacts_root().join(job_id).join("report.md")
    }

    /// Creates `<app_data>/research/estado.sqlite` with the REAL
    /// Investigations schema (`EstadoDb::abrir` applies the engine's own
    /// migrations), then drops the writer: every later access opens its own
    /// connection, exactly like the production snapshot/settlement readers.
    fn create_research_state(&self) {
        let path = self.research_state_path();
        EstadoDb::abrir(path.to_str().expect("utf-8 state path"))
            .expect("create research estado.sqlite");
    }

    /// A fresh writable connection to the research state database.
    fn open_state(&self) -> Connection {
        open_conn(&self.research_state_path())
    }

    /// Writes the engine-rendered `research/artifacts/<job>/report.md`.
    fn write_report_file(&self, job_id: &str, bytes: &[u8]) {
        let dir = self.artifacts_root().join(job_id);
        std::fs::create_dir_all(&dir).expect("job artifacts dir");
        std::fs::write(dir.join("report.md"), bytes).expect("write report.md");
    }

    // -- durable sync state (test-side reads) ------------------------------

    fn meta_value(&self, key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM sync_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .ok()
    }

    /// Pending research outbox entries (`research_outbox:`), counted by literal
    /// prefix (SQL `LIKE` would treat the underscores as wildcards).
    fn outbox_count(&self) -> i64 {
        count(
            &self.conn,
            &format!(
                "SELECT COUNT(*) FROM sync_meta WHERE substr(key, 1, {}) = '{}'",
                OUTBOX_PREFIX.len(),
                OUTBOX_PREFIX
            ),
        )
    }

    /// Durable research receive-staging rows still queued.
    fn receive_count(&self) -> i64 {
        count(
            &self.conn,
            &format!(
                "SELECT COUNT(*) FROM sync_meta WHERE substr(key, 1, {}) = '{}'",
                RECEIVE_PREFIX.len(),
                RECEIVE_PREFIX
            ),
        )
    }

    fn conflicts(&self) -> i64 {
        count(&self.conn, "SELECT COUNT(*) FROM sync_conflicts")
    }

    /// `(table_name, reason, loser_payload)` of every journaled conflict.
    fn conflict_rows(&self) -> Vec<(String, String, String)> {
        let mut stmt = self
            .conn
            .prepare("SELECT table_name, reason, loser_payload FROM sync_conflicts ORDER BY id")
            .expect("conflict rows");
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("map conflict rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect conflict rows");
        rows
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

// ---------------------------------------------------------------------------
// Research fixture builders (test-only; see the module docs on fixture SQL)
// ---------------------------------------------------------------------------

/// One terminal Investigations job exactly as the engine closes it:
/// `modo='research'`, `status='done'`, a valid `close_reason`, and non-empty
/// question/project. Timestamps are epoch SECONDS like `estado::ahora`.
fn insert_terminal_job(
    state: &Connection,
    job_id: &str,
    question: &str,
    project: &str,
    created_at: i64,
    updated_at: i64,
) {
    state
        .execute(
            "INSERT INTO jobs (id, modo, pregunta, plan_json, status, close_reason,
                               costo_acumulado, max_cost, max_llm_calls, config_snapshot,
                               corpus_snapshot_id, project, corpus, created_at, updated_at)
             VALUES (?1, 'research', ?2, NULL, 'done', 'completed', 0, NULL, NULL, '{}',
                     NULL, ?3, 'test-corpus', ?4, ?5)",
            params![job_id, question, project, created_at, updated_at],
        )
        .expect("insert terminal job");
}

fn insert_artifact(
    state: &Connection,
    id: &str,
    job_id: &str,
    kind: &str,
    version: i64,
    content_json: &str,
    created_at: i64,
) {
    state
        .execute(
            "INSERT INTO artifacts (id, job_id, tipo, path, padre, version, created_at,
                                    content_json, obsolete)
             VALUES (?1, ?2, ?3, '', NULL, ?4, ?5, ?6, 0)",
            params![id, job_id, kind, version, created_at, content_json],
        )
        .expect("insert artifact");
}

/// The `request` artifact carries the human title the envelope exposes.
fn insert_request_artifact(state: &Connection, job_id: &str, title: &str, at: i64) {
    let content = serde_json::json!({
        "title": title,
        "question": "¿Cuál es el alcance de la investigación?",
        "project": "is6-project",
    })
    .to_string();
    insert_artifact(
        state,
        &format!("fix-{job_id}-request"),
        job_id,
        "request",
        1,
        &content,
        at,
    );
}

fn report_content(marker: &str) -> String {
    serde_json::json!({
        "markdown": format!("# Informe\n\n{marker}"),
        "citations": [{ "id": "cit-1", "text": marker }],
        "provenance": [{ "kind": "fixture", "note": marker }],
    })
    .to_string()
}

fn insert_report_artifact(state: &Connection, job_id: &str, version: i64, marker: &str, at: i64) {
    insert_artifact(
        state,
        &format!("fix-{job_id}-report-{version}"),
        job_id,
        "report",
        version,
        &report_content(marker),
        at,
    );
}

/// A terminal REPORT EDIT on one device: the previous report artifact becomes
/// obsolete and a newer version carries the divergent structured report.
fn edit_report_artifact(state: &Connection, job_id: &str, version: i64, marker: &str, at: i64) {
    state
        .execute(
            "UPDATE artifacts SET obsolete = 1
              WHERE job_id = ?1 AND tipo = 'report' AND obsolete = 0",
            params![job_id],
        )
        .expect("obsolete previous report");
    insert_report_artifact(state, job_id, version, marker, at);
}

/// The `archive` artifact: a de-duplicated source manifest whose entries
/// reference corpus chunks/items that do NOT exist on this device. They must
/// travel as provenance only — never fabricate `sources`/`evidence` rows.
fn insert_archive_artifact(state: &Connection, job_id: &str, at: i64) {
    let content = serde_json::json!({
        "evidence": [
            {
                "id": "ev-missing-chunk",
                "chunk_id": "chunk-no-existe-1",
                "item_id": "item-no-existe-1",
                "text_hash": sha256_hex(b"missing-source-text"),
                "title": "Fuente ausente",
            },
            {
                "id": "ev-external",
                "provenance": "external",
                "title": "Fuente externa",
            },
        ]
    })
    .to_string();
    insert_artifact(
        state,
        &format!("fix-{job_id}-archive"),
        job_id,
        "archive",
        1,
        &content,
        at,
    );
}

/// One coalesced research outbox entry exactly as `research_capture` stores it
/// (`StoredOutboxEntry`: `op`, `changed_at`, canonical `generation`). The
/// internal capture module is `pub(crate)`, so the test writes the durable
/// shape directly — for a `U` edit it OWNS `changed_at` (the LWW adjudication
/// input) and for a `D` it enqueues the tombstone of a locally deleted job.
fn enqueue_outbox(device: &Device, job_id: &str, op: &str, changed_at: i64) {
    let value = serde_json::json!({
        "op": op,
        "changed_at": changed_at,
        "generation": Uuid::new_v4().to_string(),
    })
    .to_string();
    meta_set(&device.conn, &format!("{OUTBOX_PREFIX}{job_id}"), &value)
        .expect("research outbox entry");
}

/// The local half of a real job delete: the same rows (and rendered
/// `report.md`) the Investigations engine removes. The production hook chain
/// (`begin_delete_intent` → engine delete → `settle_delete_intent`) is
/// `pub(crate)` and separately covered by the lib tests; the test then enqueues
/// the exact durable `D` tombstone itself (see `enqueue_outbox`).
fn delete_job_locally(device: &Device, job_id: &str) {
    let state = device.open_state();
    state
        .execute("DELETE FROM artifacts WHERE job_id = ?1", params![job_id])
        .expect("delete job artifacts");
    state
        .execute("DELETE FROM jobs WHERE id = ?1", params![job_id])
        .expect("delete job row");
    drop(state);
    match std::fs::remove_file(device.report_path(job_id)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove local report.md: {error}"),
    }
}

// ---------------------------------------------------------------------------
// Projection assertions
// ---------------------------------------------------------------------------

/// The readable terminal projection contract: exactly one job plus exactly the
/// deterministic `request`/`report`/`archive` artifacts, and NOTHING else — no
/// active or billable rows are ever fabricated, and the missing-source manifest
/// entries stay provenance-only (inside the archive artifact, never in the
/// `sources`/`evidence` ledger tables).
fn assert_projection_shape(device: &Device, job_id: &str) {
    let state = device.open_state();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM jobs"),
        1,
        "exactly one projected job"
    );
    let mut kinds = Vec::new();
    {
        let mut stmt = state
            .prepare("SELECT tipo FROM artifacts WHERE job_id = ?1 ORDER BY tipo")
            .expect("artifact kinds");
        let rows = stmt
            .query_map(params![job_id], |row| row.get::<_, String>(0))
            .expect("map artifact kinds")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect artifact kinds");
        kinds.extend(rows);
    }
    assert_eq!(
        kinds,
        vec![
            "archive".to_string(),
            "report".to_string(),
            "request".to_string()
        ],
        "the projection carries exactly request/report/archive artifacts"
    );

    // Missing-source manifest entries are provenance-only and no job state is
    // ever fabricated (PROTOCOL: execution state and the billable ledger never
    // travel).
    for (table, label) in [
        ("sources", "corpus sources"),
        ("evidence", "corpus evidence"),
        ("claims", "claims"),
        ("verification_runs", "verification runs"),
        ("llm_calls", "billable LLM calls"),
        ("queries", "billable queries"),
        ("stages", "execution stages"),
        ("stage_dependencies", "stage dependencies"),
        ("job_events", "transient events"),
        ("human_decisions", "human gates"),
        ("memories", "global memories"),
    ] {
        assert_eq!(
            count(&state, &format!("SELECT COUNT(*) FROM {table}")),
            0,
            "the projection fabricates no {label}"
        );
    }
    drop(state);
}

/// The projected job summary + report artifact + `report.md` bytes/hash.
fn assert_projected_job(
    device: &Device,
    job_id: &str,
    title: &str,
    report_marker: &str,
    report_version: i64,
    report_bytes: &[u8],
) {
    assert_projection_shape(device, job_id);
    let state = device.open_state();
    let (modo, status, close_reason, pregunta): (String, String, String, String) = state
        .query_row(
            "SELECT modo, status, close_reason, pregunta FROM jobs WHERE id = ?1",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("projected job row");
    assert_eq!(
        modo, "research",
        "the projected job keeps its Investigations kind"
    );
    assert_eq!(
        status, "done",
        "the projected job keeps its terminal status"
    );
    assert_eq!(
        close_reason, "completed",
        "the projected job keeps its valid close_reason"
    );
    assert!(
        pregunta.contains("alcance de la investigación"),
        "the projected question is readable, got {pregunta:?}"
    );

    let request_title: String = state
        .query_row(
            "SELECT content_json FROM artifacts WHERE job_id = ?1 AND tipo = 'request'",
            params![job_id],
            |row| row.get(0),
        )
        .map(|raw: String| {
            let value: serde_json::Value =
                serde_json::from_str(&raw).expect("request artifact json");
            value
                .get("title")
                .and_then(|title| title.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .expect("projected request artifact");
    assert_eq!(
        request_title, title,
        "the projected request artifact carries the title"
    );

    let (report_version_seen, report_json): (i64, String) = state
        .query_row(
            "SELECT version, content_json FROM artifacts
              WHERE job_id = ?1 AND tipo = 'report'",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("projected report artifact");
    assert_eq!(
        report_version_seen, report_version,
        "the projected report keeps the winning artifact version"
    );
    let report: serde_json::Value =
        serde_json::from_str(&report_json).expect("projected report json");
    assert_eq!(
        report
            .get("markdown")
            .and_then(|markdown| markdown.as_str())
            .unwrap_or_default(),
        format!("# Informe\n\n{report_marker}"),
        "the projected report carries the winning content"
    );

    // The archive artifact keeps the manifest entries verbatim (provenance),
    // including the missing-source references.
    let archive_json: String = state
        .query_row(
            "SELECT content_json FROM artifacts WHERE job_id = ?1 AND tipo = 'archive'",
            params![job_id],
            |row| row.get(0),
        )
        .expect("projected archive artifact");
    let archive: serde_json::Value =
        serde_json::from_str(&archive_json).expect("projected archive json");
    let evidence = archive
        .get("evidence")
        .and_then(|evidence| evidence.as_array())
        .expect("archive evidence array");
    assert_eq!(
        evidence.len(),
        2,
        "the source manifest travels as provenance entries, got {archive_json:?}"
    );
    drop(state);

    // The declared report file is installed byte-verified.
    let installed = std::fs::read(device.report_path(job_id)).expect("projected report.md");
    assert_eq!(
        installed, report_bytes,
        "the projected report.md is the exact winner render"
    );
    assert_eq!(
        sha256_hex(&installed),
        sha256_hex(report_bytes),
        "the projected report.md matches its manifest hash"
    );
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// IS6.1 — terminal job created on A pushes its report blob and the whole
/// aggregate projects on B: capability discovery, since-zero catch-up, verified
/// blob download and the readable estado projection. Repeats and reopens never
/// duplicate anything and never fabricate active/billable rows.
#[tokio::test]
async fn two_device_terminal_job_push_blob_download_and_estado_projection() {
    let Some(server) = TestServer::start().await else {
        server_skip_notice();
        return;
    };
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_research_capability().await;
    a.create_research_state();
    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    b.assert_research_capability().await;
    b.create_research_state();

    // A closes one Investigations job entirely locally: fixture terminal row,
    // request/report/archive artifacts and the engine-rendered report.md.
    let job = "is6-create-job";
    let title = "Acta IS6";
    let marker = "Texto del informe base A";
    let report_bytes = format!("# Informe\n\n{marker}\n").into_bytes();
    let report_sha = sha256_hex(&report_bytes);
    let at = now_secs();
    {
        let state = a.open_state();
        insert_terminal_job(
            &state,
            job,
            "¿Cuál es el alcance de la investigación?",
            "is6-project",
            at,
            at,
        );
        insert_request_artifact(&state, job, title, at);
        insert_report_artifact(&state, job, 1, marker, at);
        insert_archive_artifact(&state, job, at);
    }
    a.write_report_file(job, &report_bytes);

    converge(&a, &b, 4).await;

    // A pushed its terminal snapshot and the report blob travelled with it.
    assert_eq!(
        a.outbox_count(),
        0,
        "A's terminal job is pushed and acknowledged"
    );
    if let Some(store) = server.store_path(&a.account_id, &report_sha) {
        let stored = std::fs::read(&store).expect("report blob stored on the server");
        assert_eq!(
            sha256_hex(&stored),
            report_sha,
            "the server holds the verified report blob beside the envelope"
        );
    }

    // B discovered the exact capability and completed the epoch catch-up.
    let capability = b
        .meta_value("research_capability")
        .expect("capability record");
    assert!(
        capability.contains("\"advertised\":true"),
        "B recorded the research-envelope-v1 capability, got {capability:?}"
    );
    assert!(
        b.meta_value("research_catchup_epoch").is_some(),
        "B recorded the epoch's since-zero research catch-up"
    );

    // B projected exactly one readable job with the verified report.md.
    assert_projected_job(&b, job, title, marker, 1, &report_bytes);

    // Repeat + reopen cycles never duplicate the job, its artifacts or the
    // report file, and nothing stays pending anywhere.
    a.reopen();
    b.reopen();
    converge(&a, &b, 3).await;
    assert_projection_shape(&b, job);
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM jobs"),
        1,
        "B keeps exactly one job after reopen"
    );
    assert_eq!(
        count(&a.open_state(), "SELECT COUNT(*) FROM jobs"),
        1,
        "A keeps exactly one job after reopen"
    );
    assert_eq!(a.outbox_count(), 0, "A holds no pending research work");
    assert_eq!(b.outbox_count(), 0, "B holds no pending research work");
    assert_eq!(b.receive_count(), 0, "B's receive queue is drained");
    assert_eq!(b.conflicts(), 0, "a clean create produces no conflicts");
    assert_eq!(a.conflicts(), 0, "a clean create produces no conflicts");
}

/// IS6.2 — divergent terminal report edits queued on BOTH devices before any
/// push, `changed_at` arranged so the FIRST push wins the LWW and the SECOND is
/// adjudicated `lww_lost`. The losing device keeps exactly one `sync_conflicts`
/// entry for `research_envelopes`, the winner projection is readable, and
/// repeat/reopen cycles never duplicate the conflict, the job or its artifacts.
#[tokio::test]
async fn two_device_divergent_report_edits_adjudicate_lww_with_one_conflict() {
    let Some(server) = TestServer::start().await else {
        server_skip_notice();
        return;
    };
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_research_capability().await;
    a.create_research_state();
    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    b.assert_research_capability().await;
    b.create_research_state();

    // Shared base: A creates the terminal job and B receives its projection.
    let job = "is6-lww-job";
    let title = "Acta IS6 LWW";
    let base_marker = "Texto del informe base";
    let base_bytes = format!("# Informe\n\n{base_marker}\n").into_bytes();
    let at = now_secs();
    {
        let state = a.open_state();
        insert_terminal_job(
            &state,
            job,
            "¿Cuál es el alcance de la investigación?",
            "is6-project",
            at,
            at,
        );
        insert_request_artifact(&state, job, title, at);
        insert_report_artifact(&state, job, 1, base_marker, at);
        insert_archive_artifact(&state, job, at);
    }
    a.write_report_file(job, &base_bytes);
    converge(&a, &b, 4).await;
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM jobs"),
        1,
        "both devices share the base job"
    );

    // Divergent terminal report edits queued on BOTH devices before any push.
    // The outbox shape is written directly (the capture hooks are `pub(crate)`
    // and covered by the lib tests) because the test OWNS `changed_at`: the
    // later timestamp belongs to the first push, so the first push wins the
    // server LWW and the second is adjudicated `lww_lost`.
    let now = now_ms();
    let winner_changed_at = now - 1_000;
    let loser_changed_at = now - 60_000;
    let winner_marker = "Texto del informe A2";
    let loser_marker = "Texto del informe B2";
    let winner_bytes = format!("# Informe\n\n{winner_marker}\n").into_bytes();
    let loser_bytes = format!("# Informe\n\n{loser_marker}\n").into_bytes();
    {
        let state = a.open_state();
        edit_report_artifact(&state, job, 2, winner_marker, at);
    }
    a.write_report_file(job, &winner_bytes);
    enqueue_outbox(&a, job, "U", winner_changed_at);
    {
        let state = b.open_state();
        edit_report_artifact(&state, job, 2, loser_marker, at);
    }
    b.write_report_file(job, &loser_bytes);
    enqueue_outbox(&b, job, "U", loser_changed_at);

    // First push wins: A uploads its report blob and its envelope is applied.
    a.sync().await;
    assert_eq!(
        a.outbox_count(),
        0,
        "the first (winning) push is applied and acknowledged"
    );

    // Second push is adjudicated `lww_lost`. B's receive of the winner is
    // retained first: the no-clobber report install never replaces B's
    // divergent local render (`ExistingTargetDifferent`), so B's outbox keeps a
    // stale `base_seq` and the server LWW resolves it against A's later
    // `changed_at`. The `research conflict for ... stays pending` warning is
    // emitted ONLY by the validated `lww_lost` settlement.
    let warnings = Mutex::new(Vec::new());
    b.sync_capturing(&warnings).await;
    let captured: Vec<String> = warnings.lock().expect("warnings").clone();
    assert!(
        captured
            .iter()
            .any(|warning| warning.contains(&format!("research conflict for {job} stays pending"))),
        "the second push is adjudicated lww_lost (settled by the lww-lost winner \
         machinery), warnings: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|warning| warning.contains(&format!("research receive for {job} stays pending"))),
        "the winner row is retained while its report cannot install, warnings: {captured:?}"
    );
    assert_eq!(
        b.outbox_count(),
        1,
        "the adjudicated generation is retained while its winner cannot settle"
    );
    assert_eq!(
        b.conflicts(),
        0,
        "nothing is overwritten while the winner is still retained"
    );

    // The user discards the losing device's unverified local render (the
    // no-clobber install never replaces a differing report.md by design); the
    // retained winner row then settles through the receive/conflict machinery.
    std::fs::remove_file(b.report_path(job)).expect("discard losing local render");

    converge(&a, &b, 4).await;

    // Exactly ONE `sync_conflicts` entry for `research_envelopes`, carrying the
    // losing manuscript; the winner projection is readable on B.
    let conflicts = b.conflict_rows();
    assert_eq!(
        conflicts.len(),
        1,
        "the losing device keeps exactly one sync_conflicts entry, got {conflicts:?}"
    );
    assert_eq!(
        conflicts[0].0, "research_envelopes",
        "the conflict belongs to the research wire table"
    );
    assert_eq!(
        conflicts[0].1, "research_divergent_upsert",
        "a divergent terminal upsert is what was adjudicated"
    );
    assert!(
        conflicts[0].2.contains(loser_marker),
        "the conflict preserves the losing report, got {:?}",
        conflicts[0].2
    );
    assert_projected_job(&b, job, title, winner_marker, 2, &winner_bytes);

    // Repeat + reopen cycles never duplicate the conflict, the job or its
    // artifacts, and both devices drain their research work.
    a.reopen();
    b.reopen();
    converge(&a, &b, 3).await;
    assert_eq!(
        b.conflict_rows().len(),
        1,
        "a replay never duplicates the conflict"
    );
    assert_projection_shape(&b, job);
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM jobs"),
        1,
        "a replay never duplicates the job"
    );
    assert_eq!(
        count(&a.open_state(), "SELECT COUNT(*) FROM jobs"),
        1,
        "A keeps exactly one job after reopen"
    );
    assert_eq!(a.outbox_count(), 0, "A holds no pending research work");
    assert_eq!(b.outbox_count(), 0, "B's adjudicated generation is settled");
    assert_eq!(b.receive_count(), 0, "B's receive queue is drained");
    assert_eq!(a.conflicts(), 0, "the winning device journals nothing");
}

/// IS6.3 — a terminal job deleted locally on A plus a `D` outbox tombstone:
/// the peer deletes its projection (rows + report.md) and repeated/reopened
/// cycles never resurrect it. The missing-source manifest entries stayed
/// provenance-only while the projection existed (the journaled loser envelope
/// is the proof: it is the canonical snapshot rebuilt from B's projection).
#[tokio::test]
async fn two_device_delete_tombstone_removes_peer_projection_without_resurrection() {
    let Some(server) = TestServer::start().await else {
        server_skip_notice();
        return;
    };
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_research_capability().await;
    a.create_research_state();
    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    b.assert_research_capability().await;
    b.create_research_state();

    // Shared base: A creates the terminal job and B receives its projection.
    let job = "is6-delete-job";
    let title = "Acta IS6 borrado";
    let marker = "Texto del informe borrable";
    let report_bytes = format!("# Informe\n\n{marker}\n").into_bytes();
    let at = now_secs();
    {
        let state = a.open_state();
        insert_terminal_job(
            &state,
            job,
            "¿Cuál es el alcance de la investigación?",
            "is6-project",
            at,
            at,
        );
        insert_request_artifact(&state, job, title, at);
        insert_report_artifact(&state, job, 1, marker, at);
        insert_archive_artifact(&state, job, at);
    }
    a.write_report_file(job, &report_bytes);
    converge(&a, &b, 4).await;
    assert_projected_job(&b, job, title, marker, 1, &report_bytes);

    // Delete the job locally on A and enqueue the exact durable `D` tombstone
    // in the test outbox (the production delete-intent hooks are `pub(crate)`
    // and covered by the lib tests).
    delete_job_locally(&a, job);
    enqueue_outbox(&a, job, "D", now_ms() - 1_000);

    converge(&a, &b, 4).await;

    // A pushed the tombstone and B deleted the whole projection.
    assert_eq!(
        a.outbox_count(),
        0,
        "the tombstone is pushed and acknowledged"
    );
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM jobs"),
        0,
        "the peer deletes the projected job"
    );
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM artifacts"),
        0,
        "the peer deletes the projected artifacts"
    );
    assert!(
        !b.report_path(job).exists(),
        "the peer deletes the projected report.md"
    );

    // The losing projection survives as exactly one conflict: the canonical
    // loser envelope, which also proves the manifest entries never corrupted
    // the projection (a coherent snapshot re-derives from it).
    let conflicts = b.conflict_rows();
    assert_eq!(
        conflicts.len(),
        1,
        "the remote delete journals exactly one conflict, got {conflicts:?}"
    );
    assert_eq!(conflicts[0].0, "research_envelopes");
    assert_eq!(
        conflicts[0].1, "research_remote_delete",
        "a remote tombstone is what deleted the projection"
    );
    let loser: serde_json::Value =
        serde_json::from_str(&conflicts[0].2).expect("loser envelope json");
    assert_eq!(
        loser
            .get("job")
            .and_then(|job| job.get("title"))
            .and_then(|title| title.as_str())
            .unwrap_or_default(),
        title,
        "the preserved loser carries the deleted job's readable summary"
    );
    assert_eq!(
        loser
            .get("sources")
            .and_then(|sources| sources.as_array())
            .map_or(0, |sources| sources.len()),
        2,
        "the source manifest re-derived from the projection as provenance only"
    );

    // Repeated + reopened cycles never resurrect the job and never re-seed it.
    a.reopen();
    b.reopen();
    converge(&a, &b, 4).await;
    assert_eq!(
        count(&a.open_state(), "SELECT COUNT(*) FROM jobs"),
        0,
        "the deleted job never returns on A"
    );
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM jobs"),
        0,
        "the deleted job never returns on B"
    );
    assert_eq!(
        count(&b.open_state(), "SELECT COUNT(*) FROM artifacts"),
        0,
        "no artifacts are resurrected on B"
    );
    assert_eq!(a.outbox_count(), 0, "A never re-seeds the deleted job");
    assert_eq!(b.outbox_count(), 0, "B never re-seeds the deleted job");
    assert_eq!(b.receive_count(), 0, "B's receive queue is drained");
    assert_eq!(
        b.conflict_rows().len(),
        1,
        "a replay never duplicates the tombstone conflict"
    );
    assert_eq!(a.conflicts(), 0, "the deleting device journals nothing");
}
