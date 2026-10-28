//! Two-device end-to-end suite for the derived Bibliography cache channel
//! (`bibliography-derived-v1`, plan `odd/plans/plan-sync-bibliografia-derivada.md`
//! revision 7 §8) against a REAL `entropia-sync-server`.
//!
//! This is the T11 companion of `tests/sync_e2e.rs`: same harness shape, same env
//! gate, the real `HttpSyncApi`, but the rows under test are the PAID RESULTS the
//! channel moves between devices instead of the corpus. Every test is `#[ignore]`
//! and gated on `ENTROPIA_SYNC_E2E_SERVER` (the base URL of a running
//! `entropia-sync-server` that implements the channel, e.g.
//! `http://127.0.0.1:18787`). When the var is unset the test prints a skip notice
//! and returns — so a plain `cargo test` (no `--ignored`) never touches the
//! network, and `--ignored` without a server skips loudly instead of failing
//! obscurely. Run it with `--ignored --test-threads=1`: the tests share one
//! server and register fresh accounts on it.
//!
//! Each test drives REAL devices: two (or three) `Device` harnesses, each with its
//! OWN temp `app_data_dir`, its OWN SQLite archive built from the checked-in
//! `tests/fixtures/schema_full.sql` (the real application schema) plus the sync
//! schema + capture triggers, its OWN in-memory session (the token never touches
//! the OS keyring) and its OWN `sync::engine::run_cycle` against the shared
//! server. Each test registers a FRESH account (unique email per run via pid + a
//! global counter), so the suite is independent and re-runnable.
//!
//! Paid results are written through the PUBLIC write helpers the paid paths call
//! after a provider answer (`sync::derived::cache_paid_vector` and
//! `sync::derived::cache_paid_ocr_page`), and read back through the PUBLIC readers
//! those paths consult before paying (`sync::derived::cached_vectors` and
//! `sync::derived::cached_ocr_pages`). A hit there is exactly "this device needs
//! no paid OpenRouter or OCR call for this row" (S3).

#![cfg(test)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use entropia_desktop_lib::sync::capture::{ensure_capture, SYNCED_TABLES};
use entropia_desktop_lib::sync::derived::{
    cache_paid_ocr_page, cache_paid_vector, cached_ocr_pages, cached_vectors, capability_state,
    request_incremental_pull, CachedOcrAnswer, DerivedCapability,
};
use entropia_desktop_lib::sync::engine::{read_schema_tag, run_cycle};
use entropia_desktop_lib::sync::http::{HttpSyncApi, LoginRequest, RegisterRequest, SyncApi};
use entropia_desktop_lib::sync::session::{clear_sync_state, meta_get, meta_set};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// The full application schema fixture (the same file `tests/sync_e2e.rs` and the
/// in-tree unit tests load). Included directly here because the `test_support`
/// helper is `#[cfg(test)]` inside the lib and not reachable from an external
/// integration crate.
const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");

/// The env var carrying the base URL of a running server. Unset ⇒ skip.
const SERVER_ENV: &str = "ENTROPIA_SYNC_E2E_SERVER";

/// Global per-process counter so every account email in a run is unique even
/// within a single test (the account-isolation scenario registers twice).
static EMAIL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A password that satisfies the server's ≥10-char rule (PROTOCOL auth).
const PASSWORD: &str = "e2e-password-123";

// The content under test. Hashes are realistic hex (lowercase, no `:`) because
// the wire keys join them with `:` and the server rejects a key component that
// contains one.
/// The embedding contract (provider/model/width/chunking) A pays under.
const CONTRACT: &str = "5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c";
/// Width of every vector in this suite.
const DIMENSIONS: i64 = 8;
/// The versioned OCR contract (T2, D3 working value shape).
const OCR_CONTRACT: &str = "ocr-v1:glm:glm-ocr:p1";

/// Reads the server base URL from the environment, or returns `None` (the caller
/// prints a skip notice and returns). Trimmed of any trailing slash.
fn server_url() -> Option<String> {
    match std::env::var(SERVER_ENV) {
        Ok(url) if !url.trim().is_empty() => Some(url.trim().trim_end_matches('/').to_string()),
        _ => None,
    }
}

/// The skip guard every test runs first. Returns the server URL or prints a
/// clear notice and signals the test to return early.
macro_rules! server_or_skip {
    () => {
        match server_url() {
            Some(url) => url,
            None => {
                eprintln!(
                    "[derived-e2e] SKIP: set {SERVER_ENV}=<server-url> to run the two-device \
                     derived-cache suite (e.g. http://127.0.0.1:18787)."
                );
                return;
            }
        }
    };
}

/// Builds a unique account email for this run: `derived-e2e-{pid}-{counter}@entropia.test`.
fn fresh_email() -> String {
    let pid = std::process::id();
    let n = EMAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("derived-e2e-{pid}-{n}@entropia.test")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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

// ---------------------------------------------------------------------------
// Device harness
// ---------------------------------------------------------------------------

/// One simulated device: a temp app-data dir, a SQLite archive carrying the full
/// app schema + sync schema + capture triggers, an HTTP client against the shared
/// server, and an in-memory session (token never touches the OS keyring).
struct Device {
    /// Kept alive so the temp dir is not deleted while the test runs.
    _tempdir: tempfile::TempDir,
    app_data_dir: PathBuf,
    /// The device's OWN connection — never shared, configured exactly like the
    /// sync module's `open_sync_connection` (WAL, FK on, busy_timeout).
    conn: Connection,
    api: HttpSyncApi,
    server_url: String,
    token: String,
    device_id: String,
    account_id: String,
}

impl Device {
    /// Creates a fresh device: temp dir, schema fixture loaded, sync schema +
    /// triggers ensured. No session yet (call [`Device::login`] /
    /// [`Device::register_and_login`]).
    fn new(server_url: &str) -> Device {
        let tempdir = tempfile::tempdir().expect("create temp app_data_dir");
        let app_data_dir = tempdir.path().to_path_buf();
        std::fs::create_dir_all(app_data_dir.join("assets")).expect("create assets dir");
        let db_path = app_data_dir.join("entropia.sqlite");

        let conn = open_conn(&db_path);
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("apply schema fixture");
        // Record a migration head so `read_schema_tag` returns a real tag and the
        // server's schema_tag lifecycle is exercised (PROTOCOL "schema_tag").
        conn.execute(
            "INSERT INTO _migrations(name, applied_at) VALUES ('0028_vec_assets_embedding_contract', ?1)",
            rusqlite::params![now_ms()],
        )
        .expect("seed migration head");
        ensure_capture(&conn).expect("ensure sync schema + triggers");

        let api = HttpSyncApi::new(server_url).expect("build HTTP client");
        Device {
            _tempdir: tempdir,
            app_data_dir,
            conn,
            api,
            server_url: server_url.to_string(),
            token: String::new(),
            device_id: String::new(),
            account_id: String::new(),
        }
    }

    /// Registers a brand-new account and logs this device into it. Returns the
    /// `(email, account_id)` so a second device can log into the same account.
    async fn register_and_login(&mut self) -> (String, String) {
        let email = fresh_email();
        self.api
            .register(RegisterRequest {
                email: email.clone(),
                password: PASSWORD.to_string(),
            })
            .await
            .expect("register account (server must run with SYNC_REGISTRATION_OPEN=true)");
        let account_id = self.login(&email).await;
        (email, account_id)
    }

    /// Logs this device into an existing account, persisting the session exactly
    /// like `sync_login` does (minus the keyring): token in-memory, session meta
    /// in `sync_meta`, capture ON. The session incarnation is minted here because
    /// `write_sync_session` does it in production and the derived channel refuses
    /// to run without one (`read_scope` requires it). Returns the `account_id`.
    async fn login(&mut self, email: &str) -> String {
        let resp = self
            .api
            .login(LoginRequest {
                email: email.to_string(),
                password: PASSWORD.to_string(),
                device_name: format!("derived-e2e-{}", std::process::id()),
                platform: "test".to_string(),
            })
            .await
            .expect("login");
        self.token = resp.device_token;
        self.device_id = resp.device_id;
        self.account_id = resp.account_id.clone();

        meta_set(&self.conn, "server_url", &self.server_url).unwrap();
        meta_set(&self.conn, "account_id", &resp.account_id).unwrap();
        meta_set(&self.conn, "account_email", email).unwrap();
        meta_set(&self.conn, "device_id", &self.device_id).unwrap();
        meta_set(&self.conn, "capture_enabled", "1").unwrap();
        meta_set(
            &self.conn,
            "sync_session_incarnation",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        resp.account_id
    }

    /// Logs this device out exactly like `sync_logout` does (minus the keyring):
    /// the production [`clear_sync_state`] wipe. The paid cache keeps its rows and
    /// only resets `pushed` (plan §2.9), so a re-login of the same account
    /// re-pushes them — the idempotency scenario.
    fn logout(&mut self) {
        clear_sync_state(&self.conn).expect("clear sync state");
        self.token.clear();
    }

    /// Runs one full sync cycle against the server (PROTOCOL flow, derived phase
    /// included). Panics with a descriptive message on a cycle error so test
    /// failures are legible.
    async fn sync(&self) {
        let warn = |msg: String| eprintln!("[derived-e2e warn] {msg}");
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

    /// Convenience: run the cycle twice, so the device both flushes what it owes
    /// and re-reads what its peer pushed after its own last pull.
    async fn sync_twice(&self) {
        self.sync().await;
        self.sync().await;
    }
}

/// Opens a connection with the sync module's standard pragmas (WAL, FK on,
/// busy_timeout) — the same configuration `open_sync_connection` applies.
fn open_conn(db_path: &Path) -> Connection {
    let conn = Connection::open(db_path).expect("open device db");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )
    .expect("configure pragmas");
    conn
}

// ---------------------------------------------------------------------------
// Paid-result write helpers (the same helpers the paid paths call)
// ---------------------------------------------------------------------------

/// Seeds the immutable embedding contract row the paid vector is computed under.
/// `cache_paid_vector` mirrors it into `sync_derived_contracts` (and the mirror is
/// what travels on the wire), exactly as the embedder's publishing transaction
/// does.
fn seed_embedding_contract(conn: &Connection, contract_hash: &str, dimensions: i64) {
    conn.execute(
        "INSERT OR IGNORE INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES (?1, 'openrouter', 'text-embedding-3-large', ?2, 'chunks-v1:800:100', ?3)",
        rusqlite::params![contract_hash, dimensions, now_ms()],
    )
    .expect("seed embedding contract");
}

/// A deterministic `dimensions`-wide f32 LE blob: finite values, distinct per row
/// so a byte-level comparison cannot pass by accident.
fn vector_blob(dimensions: usize, base: f32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(dimensions * 4);
    for index in 0..dimensions {
        bytes.extend_from_slice(&(base + index as f32 / 8.0).to_le_bytes());
    }
    bytes
}

/// Records one paid embedding through the public helper the paid path uses,
/// returning the exact blob it stored.
fn paid_vector(
    device: &Device,
    contract_hash: &str,
    input_hash: &str,
    dimensions: usize,
    base: f32,
) -> Vec<u8> {
    let embedding = vector_blob(dimensions, base);
    cache_paid_vector(
        &device.conn,
        contract_hash,
        input_hash,
        dimensions as i64,
        &embedding,
    )
    .expect("cache a paid vector");
    embedding
}

/// Records one paid OCR answer through the public helper the OCR path uses,
/// empty answers included.
fn paid_ocr_page(
    device: &Device,
    source_hash: &str,
    page_number: i64,
    ocr_contract: &str,
    outcome: &str,
    text: &str,
) {
    cache_paid_ocr_page(
        &device.conn,
        source_hash,
        page_number,
        ocr_contract,
        outcome,
        text,
    )
    .expect("cache a paid OCR page");
}

// ---------------------------------------------------------------------------
// Domain write helpers (simulate the app writing corpus rows; triggers fire)
// ---------------------------------------------------------------------------

fn insert_collection(conn: &Connection, id: &str, name: &str) {
    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES(?1,?2,?3,?3)",
        rusqlite::params![id, name, now_ms()],
    )
    .expect("insert collection");
}

fn insert_item(conn: &Connection, id: &str, title: &str, collection_id: &str) {
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
        rusqlite::params![id, title, collection_id, now_ms()],
    )
    .expect("insert item");
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count query")
}

/// Rows still awaiting a push for `account`: the derived outbox depth the cycle
/// reports as `pending_rows`.
fn pending_rows(conn: &Connection, account: &str) -> i64 {
    conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM sync_derived_vectors WHERE account_id = ?1 AND pushed = 0)
          + (SELECT COUNT(*) FROM sync_derived_ocr_pages WHERE account_id = ?1 AND pushed = 0)",
        [account],
        |row| row.get(0),
    )
    .expect("derived outbox depth")
}

/// One row of [`derived_vectors`]'s canonical projection:
/// `(account, contract, input, dimensions, sha256(embedding), created_at)`.
type DerivedVectorRow = (String, String, String, i64, String, i64);

/// One row of [`derived_ocr_rows`]'s canonical projection:
/// `(account, source, page, contract, outcome, text, text_hash, created_at)`.
type DerivedOcrRow = (String, String, i64, String, String, String, String, i64);

/// Every derived vector of an archive in a canonical, device-independent
/// projection: `(account, contract, input, dimensions, sha256(embedding),
/// created_at)`. `origin` and `pushed` are deliberately excluded — they are
/// device-local bookkeeping (`local` on the payer, `remote` on the reader), never
/// content — so equal projections mean the reader holds exactly the payer's rows,
/// blob included.
fn derived_vectors(conn: &Connection) -> Vec<DerivedVectorRow> {
    let mut statement = conn
        .prepare(
            "SELECT account_id, contract_hash, input_hash, dimensions, embedding, created_at
               FROM sync_derived_vectors
              ORDER BY account_id, contract_hash, input_hash",
        )
        .expect("prepare derived vector projection");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .expect("query derived vectors")
        .map(|row| row.expect("read derived vector"))
        .collect::<Vec<_>>();
    rows.into_iter()
        .map(|(account, contract, input, dimensions, blob, created_at)| {
            (
                account,
                contract,
                input,
                dimensions,
                sha256_hex(&blob),
                created_at,
            )
        })
        .collect()
}

/// Every derived OCR page of an archive in a canonical projection:
/// `(account, source, page, contract, outcome, text, text_hash, created_at)`.
/// The raw provider answer travels as-is, so the text itself is compared here.
fn derived_ocr_rows(conn: &Connection) -> Vec<DerivedOcrRow> {
    let mut statement = conn
        .prepare(
            "SELECT account_id, source_hash, page_number, ocr_contract, outcome, text_content,
                    text_hash, created_at
               FROM sync_derived_ocr_pages
              ORDER BY account_id, source_hash, page_number, ocr_contract",
        )
        .expect("prepare derived OCR projection");
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .expect("query derived OCR pages")
        .map(|row| row.expect("read derived OCR page"))
        .collect()
}

/// A canonical, order-stable projection of one synced corpus table: every
/// non-generated column, sorted by the first projected column. Copied from
/// `tests/sync_e2e.rs` so the corpus assertion is the same one that suite makes.
///
/// `collections.updated_at` is excluded deliberately by that exact pair rather
/// than by column name: migration 0027 makes it the canonical last-activity
/// timestamp, stamped with the applying device's own clock by the
/// `collection_activity_*` triggers, so two devices legitimately hold different
/// values.
fn canonical_table(conn: &Connection, table: &str) -> Vec<serde_json::Value> {
    let device_local = |c: &str| table == "collections" && c == "updated_at";

    let cols = non_generated_columns(conn, table);
    let projected_cols: Vec<String> = cols.into_iter().filter(|c| !device_local(c)).collect();
    let col_list = projected_cols
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {col_list} FROM \"{table}\" ORDER BY 1");
    let mut stmt = conn.prepare(&sql).expect("prepare canonical query");
    let rows = stmt
        .query_map([], |row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in projected_cols.iter().enumerate() {
                let value = column_to_json(row, i);
                obj.insert(col.clone(), value);
            }
            Ok(serde_json::Value::Object(obj))
        })
        .expect("query canonical rows");
    rows.map(|r| r.expect("read canonical row")).collect()
}

/// Reads a rusqlite column into a JSON value (text/int/real/null/blob-as-hex).
fn column_to_json(row: &rusqlite::Row, idx: usize) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    match row.get_ref(idx).expect("column ref") {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::Number(i.into()),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueRef::Text(t) => serde_json::Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => serde_json::Value::String(sha256_hex(b)),
    }
}

/// Non-generated columns of a table (`PRAGMA table_xinfo`, `hidden = 0`).
fn non_generated_columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_xinfo(\"{table}\")"))
        .expect("prepare table_xinfo");
    let cols = stmt
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let hidden: i64 = row.get(6)?;
            Ok((name, hidden))
        })
        .expect("query table_xinfo");
    cols.filter_map(|r| {
        let (name, hidden) = r.expect("read xinfo row");
        if hidden == 0 {
            Some(name)
        } else {
            None
        }
    })
    .collect()
}

/// Asserts two devices have converged on every corpus table: the derived channel
/// must never disturb the ordinary sync.
fn assert_corpus_converged(a: &Device, b: &Device) {
    for table in SYNCED_TABLES {
        let ta = canonical_table(&a.conn, table);
        let tb = canonical_table(&b.conn, table);
        assert_eq!(
            ta, tb,
            "table `{table}` diverged between devices\nA = {ta:#?}\nB = {tb:#?}"
        );
    }
}

/// Runs alternating cycles (payer first) until `converged` holds, at most `bound`
/// rounds, and returns. Between rounds the READER asks for an incremental pull
/// through the same public hook the pull-before-pay gate uses, so the channel's
/// 60 s minimum spacing between incremental pulls never makes the suite wait or
/// flake (the since-zero catch-up needs no request at all). The caller asserts the
/// exact state afterwards, so a failure shows the diff instead of a bare "did not
/// converge".
async fn converge_derived(
    payer: &Device,
    reader: &Device,
    bound: usize,
    converged: impl Fn() -> bool,
) {
    for round in 0..bound {
        payer.sync().await;
        if converged() {
            return;
        }
        if round > 0 {
            request_incremental_pull(&reader.conn).expect("request an incremental derived pull");
        }
        reader.sync().await;
        if converged() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Scenarios (plan §8) — one test each
// ---------------------------------------------------------------------------

/// (a) A pays for two vectors and two OCR pages (one empty) and syncs; B logs
/// into the same account, discovers the capability, catches up, and its cache
/// answers exactly A's rows — blob and text byte-identical (S3).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn paid_results_written_on_a_reach_b_byte_identical() {
    let url = server_or_skip!();
    let profile_input = sha256_hex(b"profile:sha256-of-the-book");
    let chunk_input = sha256_hex(b"chunk:page-7-paragraph-2");
    let source_hash = sha256_hex(b"%PDF-1.7 fake book bytes");

    let mut a = Device::new(&url);
    let (email, account) = a.register_and_login().await;

    // A pays: contract row, two vectors, two OCR pages (one empty answer, which
    // was charged too, so it must travel).
    seed_embedding_contract(&a.conn, CONTRACT, DIMENSIONS);
    let profile_vector = paid_vector(&a, CONTRACT, &profile_input, DIMENSIONS as usize, 0.25);
    let chunk_vector = paid_vector(&a, CONTRACT, &chunk_input, DIMENSIONS as usize, 0.75);
    paid_ocr_page(
        &a,
        &source_hash,
        1,
        OCR_CONTRACT,
        "text",
        "página uno, pagada",
    );
    paid_ocr_page(&a, &source_hash, 2, OCR_CONTRACT, "empty", "");
    assert_eq!(
        pending_rows(&a.conn, &account),
        4,
        "the four paid rows await their first push"
    );

    // One cycle records the capability from the ordinary response, runs the
    // epoch's since-zero catch-up and pushes A's outbox — in that order.
    a.sync().await;
    assert_eq!(
        pending_rows(&a.conn, &account),
        0,
        "A pushed every paid row it owns"
    );
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_derived_contracts"),
        1,
        "the contract row travels with the first vector of its space"
    );

    // B joins the same account: capability discovery + catch-up, bounded.
    let mut b = Device::new(&url);
    b.login(&email).await;
    converge_derived(&a, &b, 4, || !derived_vectors(&b.conn).is_empty()).await;

    // The readers the paid paths consult answer A's rows, byte-identically.
    let hits = cached_vectors(
        &b.conn,
        CONTRACT,
        DIMENSIONS as usize,
        &[profile_input.clone(), chunk_input.clone()],
    );
    assert_eq!(
        hits.get(&profile_input),
        Some(&profile_vector),
        "B answers the profile vector from the cache instead of paying for it"
    );
    assert_eq!(
        hits.get(&chunk_input),
        Some(&chunk_vector),
        "B answers the chunk vector from the cache instead of paying for it"
    );
    assert_eq!(hits.len(), 2, "no extra vector answers the read");

    let pages = cached_ocr_pages(&b.conn, &source_hash, OCR_CONTRACT, &[1, 2]);
    assert_eq!(
        pages.get(&1),
        Some(&CachedOcrAnswer {
            outcome: "text".to_string(),
            text: "página uno, pagada".to_string(),
        }),
        "the page A paid for answers with A's exact text"
    );
    assert_eq!(
        pages.get(&2),
        Some(&CachedOcrAnswer {
            outcome: "empty".to_string(),
            text: String::new(),
        }),
        "an empty paid answer travels too: B must not pay again"
    );
    assert_eq!(pages.len(), 2, "no extra page answers the read");

    // ... and the whole cache holds exactly A's rows (no extra, no missing).
    assert_eq!(
        derived_vectors(&b.conn),
        derived_vectors(&a.conn),
        "B's derived vector cache equals A's, byte for byte"
    );
    assert_eq!(
        derived_ocr_rows(&b.conn),
        derived_ocr_rows(&a.conn),
        "B's derived OCR cache equals A's, text for text"
    );
    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM sync_derived_contracts"),
        1,
        "the contract row reached B"
    );
    assert_eq!(
        pending_rows(&b.conn, &account),
        0,
        "received rows are already on the server: B owes nothing"
    );
}

/// (b) Account isolation: a third device on ANOTHER account syncs against the
/// same server and sees none of the first account's derived rows (S7, plan §2.9).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn another_account_never_sees_the_derived_rows() {
    let url = server_or_skip!();
    let profile_input = sha256_hex(b"profile:other-account");
    let source_hash = sha256_hex(b"%PDF-1.7 other book");

    let mut a = Device::new(&url);
    let (_email_x, account_x) = a.register_and_login().await;
    seed_embedding_contract(&a.conn, CONTRACT, DIMENSIONS);
    let _vector = paid_vector(&a, CONTRACT, &profile_input, DIMENSIONS as usize, 1.5);
    paid_ocr_page(
        &a,
        &source_hash,
        3,
        OCR_CONTRACT,
        "text",
        "página tres, pagada",
    );
    a.sync().await;
    assert_eq!(
        pending_rows(&a.conn, &account_x),
        0,
        "account X's rows reached the channel"
    );

    // C registers its OWN account on the same server and syncs.
    let mut c = Device::new(&url);
    let (_email_c, account_c) = c.register_and_login().await;
    assert_ne!(account_c, account_x, "C must be a different account");
    c.sync_twice().await;

    // C's cache reads answer nothing: no row of X is ever used (§2.9/D6).
    assert!(
        cached_vectors(
            &c.conn,
            CONTRACT,
            DIMENSIONS as usize,
            std::slice::from_ref(&profile_input)
        )
        .is_empty(),
        "C must not answer a vector paid by another account"
    );
    assert!(
        cached_ocr_pages(&c.conn, &source_hash, OCR_CONTRACT, &[3]).is_empty(),
        "C must not answer an OCR page paid by another account"
    );
    assert!(
        derived_vectors(&c.conn).is_empty(),
        "no derived vector row of another account was installed on C"
    );
    assert!(
        derived_ocr_rows(&c.conn).is_empty(),
        "no derived OCR row of another account was installed on C"
    );

    // A's own cache is untouched by C's cycles on the shared server.
    assert_eq!(
        cached_vectors(
            &a.conn,
            CONTRACT,
            DIMENSIONS as usize,
            std::slice::from_ref(&profile_input)
        )
        .len(),
        1,
        "account X keeps its own cached result"
    );
}

/// (c) Idempotency: A logs out and back into the SAME account (the logout resets
/// `pushed` and keeps the rows), re-pushes, and the server ends with one row per
/// composite key — B still sees each key once and its pull is stable.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn repush_after_logout_leaves_no_duplicates() {
    let url = server_or_skip!();
    let profile_input = sha256_hex(b"profile:idempotent");
    let chunk_input = sha256_hex(b"chunk:idempotent");
    let source_hash = sha256_hex(b"%PDF-1.7 idempotent book");

    let mut a = Device::new(&url);
    let (email, account) = a.register_and_login().await;
    seed_embedding_contract(&a.conn, CONTRACT, DIMENSIONS);
    let _v1 = paid_vector(&a, CONTRACT, &profile_input, DIMENSIONS as usize, 2.0);
    let _v2 = paid_vector(&a, CONTRACT, &chunk_input, DIMENSIONS as usize, 2.5);
    paid_ocr_page(&a, &source_hash, 1, OCR_CONTRACT, "text", "una");
    paid_ocr_page(&a, &source_hash, 2, OCR_CONTRACT, "text", "dos");

    let mut b = Device::new(&url);
    b.login(&email).await;
    converge_derived(&a, &b, 4, || derived_vectors(&b.conn).len() == 2).await;

    let vectors_before = derived_vectors(&b.conn);
    let ocr_before = derived_ocr_rows(&b.conn);
    assert_eq!(vectors_before.len(), 2, "B received A's two vectors once");
    assert_eq!(ocr_before.len(), 2, "B received A's two OCR pages once");

    // Logout keeps the paid rows and resets their push flags (§2.9), so the same
    // account re-pushes content the server already holds.
    a.logout();
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_derived_vectors"),
        2,
        "logout must not delete paid results"
    );
    assert_eq!(
        pending_rows(&a.conn, &account),
        4,
        "logout resets `pushed` on every paid row of the account"
    );
    a.login(&email).await;
    a.sync_twice().await;
    assert_eq!(
        pending_rows(&a.conn, &account),
        0,
        "the re-login re-pushed every row and the server acknowledged it"
    );

    // B pulls again: the re-push added no key, so nothing changed.
    request_incremental_pull(&b.conn).expect("request an incremental derived pull");
    b.sync_twice().await;
    assert_eq!(
        derived_vectors(&b.conn),
        vectors_before,
        "B still sees each vector key exactly once, unchanged"
    );
    assert_eq!(
        derived_ocr_rows(&b.conn),
        ocr_before,
        "B still sees each OCR key exactly once, unchanged"
    );

    // The server itself holds one row per composite key: a since-zero read of the
    // account's channel lists every key exactly once (insert-if-absent, §4).
    let schema_tag = read_schema_tag(&b.conn).expect("read schema tag");
    let page = b
        .api
        .pull_with_bibliography_derived_v1(&b.token, &schema_tag, 0, 1000)
        .await
        .expect("read the whole derived channel of the account");
    assert!(!page.has_more, "the account's channel fits in one page");
    let ids = page
        .rows
        .iter()
        .map(|row| row.row_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids.len(),
        5,
        "one contract row, two vectors and two OCR pages: {ids:#?}"
    );
    let unique = ids.iter().collect::<HashSet<_>>();
    assert_eq!(
        unique.len(),
        ids.len(),
        "the server stored each composite key once: {ids:#?}"
    );
    // A derived pull answers derived rows only, and each of the three wire tables
    // holds exactly what A paid for (no duplicate keys, nothing missing).
    for row in &page.rows {
        assert!(
            ["derived_vectors", "derived_ocr_pages", "derived_contracts"]
                .contains(&row.table.as_str()),
            "a derived pull must answer derived rows only, got `{}`",
            row.table
        );
    }
    let per_table = |table: &str| page.rows.iter().filter(|row| row.table == table).count();
    assert_eq!(
        per_table("derived_vectors"),
        2,
        "two vector keys, once each"
    );
    assert_eq!(
        per_table("derived_ocr_pages"),
        2,
        "two OCR page keys, once each"
    );
    assert_eq!(
        per_table("derived_contracts"),
        1,
        "the contract row travelled once"
    );
}

/// (d) Corpus unaffected: a normal synced row (a collection and its item) still
/// converges between A and B in the same run as the derived channel (S3).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn corpus_rows_still_converge_while_the_channel_runs() {
    let url = server_or_skip!();
    let profile_input = sha256_hex(b"profile:corpus");

    let mut a = Device::new(&url);
    let (email, account) = a.register_and_login().await;
    seed_embedding_contract(&a.conn, CONTRACT, DIMENSIONS);
    let _vector = paid_vector(&a, CONTRACT, &profile_input, DIMENSIONS as usize, 3.5);

    // The ordinary corpus writes share the cycle with the derived phase.
    insert_collection(&a.conn, "c1", "Colección con caché");
    insert_item(&a.conn, "i1", "Documento sincronizado", "c1");

    let mut b = Device::new(&url);
    b.login(&email).await;
    converge_derived(&a, &b, 4, || {
        count(&b.conn, "SELECT COUNT(*) FROM items WHERE id='i1'") == 1
            && !derived_vectors(&b.conn).is_empty()
    })
    .await;

    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM collections WHERE id='c1'"),
        1,
        "the collection reached B"
    );
    let title: String = b
        .conn
        .query_row("SELECT title FROM items WHERE id='i1'", [], |r| r.get(0))
        .expect("item on B");
    assert_eq!(title, "Documento sincronizado");
    assert_corpus_converged(&a, &b);
    assert_eq!(
        derived_vectors(&b.conn),
        derived_vectors(&a.conn),
        "the derived row travelled in the same run"
    );
    assert_eq!(
        pending_rows(&b.conn, &account),
        0,
        "B owes nothing on either channel"
    );
}

/// (e) Capability: both devices record `bibliography-derived-v1` as Supported for
/// the epoch their first cycles synced (§2.5).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn capability_is_supported_on_both_devices_after_first_cycles() {
    let url = server_or_skip!();

    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    assert_eq!(
        capability_state(&a.conn),
        DerivedCapability::Unknown,
        "nothing is known before the first ordinary response"
    );
    a.sync().await;
    assert_eq!(
        capability_state(&a.conn),
        DerivedCapability::Supported,
        "A's first cycle recorded the advertised token for its epoch"
    );

    let mut b = Device::new(&url);
    b.login(&email).await;
    assert_eq!(
        capability_state(&b.conn),
        DerivedCapability::Unknown,
        "B has not talked to the server yet"
    );
    b.sync().await;
    assert_eq!(
        capability_state(&b.conn),
        DerivedCapability::Supported,
        "B's first cycle recorded the advertised token for its epoch"
    );

    // The verdict is bound to the epoch both devices synced: a different epoch
    // would read as Unknown and require a fresh advertisement.
    let epoch_a = meta_get(&a.conn, "server_epoch").expect("epoch meta");
    let epoch_b = meta_get(&b.conn, "server_epoch").expect("epoch meta");
    assert!(
        epoch_a.as_deref().is_some_and(|epoch| !epoch.is_empty()),
        "A persisted the server epoch"
    );
    assert_eq!(epoch_a, epoch_b, "both devices synced the same epoch");
}
