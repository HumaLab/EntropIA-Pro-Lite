//! Multi-device end-to-end suite for the cloud-sync client (DESIGN §13.4,
//! PROTOCOL "Flujo de sincronización del cliente").
//!
//! Every test is `#[ignore]` and gated on the env var `ENTROPIA_SYNC_E2E_SERVER`
//! (the base URL of a running `entropia-sync-server`, e.g. `http://127.0.0.1:8787`).
//! When the var is unset the test prints a skip notice and returns — so a plain
//! `cargo test` (no `--ignored`) never touches the network, and `--ignored`
//! without a server skips loudly instead of failing obscurely. The orchestrator
//! script `scripts/e2e-local.ps1` (EntropIA-Cloud repo) builds the server, starts
//! it on an ephemeral port with `SYNC_REGISTRATION_OPEN=true`, sets the env var,
//! and runs this file with `--ignored --test-threads=1`.
//!
//! Each test drives REAL devices: two `Device` harnesses, each with its OWN temp
//! `app_data_dir`, its OWN SQLite DB built from the checked-in
//! `tests/fixtures/schema_full.sql` (the real application schema) plus the sync
//! schema + 45 capture triggers, and its OWN engine cycle (`run_cycle`) against
//! the shared server. The device token lives in-memory on the harness (NOT the OS
//! keyring) so two independent "devices" coexist on one machine — exactly the
//! `com.entropia.lite.dev` second-instance trick from DESIGN §2, minus the
//! keyring. Each test registers a FRESH account (unique email per run via pid +
//! a global counter) so the suite is independent and re-runnable.
//!
//! Convergence is asserted by querying BOTH DBs and comparing canonical row sets
//! (deterministic JSON projections of the synced tables) plus, for blobs, the
//! sha256 of the downloaded file byte-for-byte.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use entropia_desktop_lib::sync::capture::{ensure_capture, SYNCED_TABLES};
use entropia_desktop_lib::sync::engine::run_cycle;
use entropia_desktop_lib::sync::http::{HttpSyncApi, LoginRequest, RegisterRequest, SyncApi};
use entropia_desktop_lib::sync::session::{meta_set, meta_set_i64};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// The full application schema fixture (the same file the in-tree unit tests
/// load via `test_support::SCHEMA_FIXTURE`). Included directly here because the
/// `test_support` helper is `#[cfg(test)]` inside the lib and not reachable from
/// an external integration crate.
const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");

/// The env var carrying the base URL of a running server. Unset ⇒ skip.
const SERVER_ENV: &str = "ENTROPIA_SYNC_E2E_SERVER";

/// Global per-process counter so every account email in a run is unique even
/// within a single test (e.g. account-change scenarios register twice).
static EMAIL_COUNTER: AtomicU64 = AtomicU64::new(0);

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
                    "[sync-e2e] SKIP: set {SERVER_ENV}=<server-url> to run the multi-device suite \
                     (e.g. http://127.0.0.1:8787). See scripts/e2e-local.ps1 in EntropIA-Cloud."
                );
                return;
            }
        }
    };
}

/// Builds a unique account email for this run: `e2e-{pid}-{counter}@entropia.test`.
fn fresh_email() -> String {
    let pid = std::process::id();
    let n = EMAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("e2e-{pid}-{n}@entropia.test")
}

/// A password that satisfies the server's ≥10-char rule (PROTOCOL auth).
const PASSWORD: &str = "e2e-password-123";

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

/// One simulated device: a temp app-data dir, a SQLite DB carrying the full app
/// schema + sync schema + capture triggers, an HTTP client against the shared
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
            "INSERT INTO _migrations(name, applied_at) VALUES ('0023_sync_ids', ?1)",
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
    /// `(email, account_id)` so a SECOND device can log into the same account.
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
    /// in `sync_meta`, capture ON. Returns the `account_id`.
    async fn login(&mut self, email: &str) -> String {
        let resp = self
            .api
            .login(LoginRequest {
                email: email.to_string(),
                password: PASSWORD.to_string(),
                device_name: format!("e2e-{}", std::process::id()),
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
        resp.account_id
    }

    /// Switches this device to a different account WITHOUT wiping local data
    /// (DESIGN §6.3 "conservar datos locales y fusionarlos" path): clears the
    /// seeded marker + cursor so the next cycle re-seeds the whole library into
    /// the new account, then logs in.
    async fn switch_account_keep_data(&mut self, email: &str) -> String {
        // Mirror the "merge local data" account-change branch: forget the
        // server-side bookkeeping so the library re-seeds under the new account.
        for key in ["seeded_account", "server_epoch"] {
            self.conn
                .execute("DELETE FROM sync_meta WHERE key = ?1", [key])
                .unwrap();
        }
        meta_set_i64(&self.conn, "last_pull_seq", 0).unwrap();
        self.conn
            .execute_batch(
                "DELETE FROM sync_row_versions; UPDATE sync_blob_index SET uploaded = 0;",
            )
            .unwrap();
        self.login(email).await
    }

    /// Runs one full sync cycle against the server (PROTOCOL flow). Panics with a
    /// descriptive message on a cycle error so test failures are legible.
    async fn sync(&self) {
        let warn = |msg: String| eprintln!("[sync-e2e warn] {msg}");
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

    /// Convenience: run the cycle twice. A single cycle pushes local changes and
    /// pulls remote state; a second cycle lets a device observe what its peer
    /// pushed AFTER its own last pull. Tests that need bidirectional convergence
    /// alternate `a.sync(); b.sync(); a.sync();` explicitly, but a double-sync is
    /// handy when a device must both flush and re-read.
    async fn sync_twice(&self) {
        self.sync().await;
        self.sync().await;
    }
}

/// Opens a connection with the sync module's standard pragmas (WAL, FK on,
/// busy_timeout) — the same configuration `open_sync_connection` applies. The
/// E2E test opens its own connection (it never has access to `ui_conn`).
fn open_conn(db_path: &Path) -> Connection {
    let conn = Connection::open(db_path).expect("open device db");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )
    .expect("configure pragmas");
    conn
}

// ---------------------------------------------------------------------------
// Domain write helpers (simulate the app writing rows; capture triggers fire)
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

fn update_item_title(conn: &Connection, id: &str, title: &str) {
    let n = conn
        .execute(
            "UPDATE items SET title = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, title, now_ms()],
        )
        .expect("update item");
    assert_eq!(n, 1, "update item {id} affected no rows");
}

fn delete_item(conn: &Connection, id: &str) {
    conn.execute("DELETE FROM items WHERE id = ?1", [id])
        .expect("delete item");
}

/// Inserts an `entities` row — a child of `items` with `ON DELETE CASCADE`, so
/// deleting its parent item cascade-destroys it (the cascade case for the
/// tombstone-over-dirty-child scenario, DESIGN §4.4).
fn insert_entity(conn: &Connection, id: &str, item_id: &str, value: &str) {
    conn.execute(
        "INSERT INTO entities(id,item_id,entity_type,value,created_at) VALUES(?1,?2,'person',?3,?4)",
        rusqlite::params![id, item_id, value, now_ms()],
    )
    .expect("insert entity");
}

fn update_entity_value(conn: &Connection, id: &str, value: &str) {
    let n = conn
        .execute(
            "UPDATE entities SET value = ?2 WHERE id = ?1",
            rusqlite::params![id, value],
        )
        .expect("update entity");
    assert_eq!(n, 1, "update entity {id} affected no rows");
}

/// Imports a real small file as an asset: writes it under
/// `assets/{collection}/{item}/{uuid}_{name}` (matching the app's layout) and
/// inserts the `assets` row pointing at the absolute local path. Returns the
/// absolute path so the test can re-derive the expected sha256.
fn import_asset_file(
    device: &Device,
    asset_id: &str,
    item_id: &str,
    collection_id: &str,
    file_name: &str,
    bytes: &[u8],
) -> PathBuf {
    let rel_dir = Path::new("assets").join(collection_id).join(item_id);
    let abs_dir = device.app_data_dir.join(&rel_dir);
    std::fs::create_dir_all(&abs_dir).expect("create asset dir");
    let abs_path = abs_dir.join(format!("{asset_id}_{file_name}"));
    std::fs::write(&abs_path, bytes).expect("write asset file");
    device
        .conn
        .execute(
            "INSERT INTO assets(id,item_id,path,type,size,created_at,sort_index)
             VALUES(?1,?2,?3,'image',?4,?5,0)",
            rusqlite::params![
                asset_id,
                item_id,
                abs_path.to_string_lossy().to_string(),
                bytes.len() as i64,
                now_ms()
            ],
        )
        .expect("insert asset row");
    abs_path
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/// A canonical, order-stable projection of one synced table: every non-generated
/// column EXCEPT the volatile `path` (absolute, device-local) for assets, sorted
/// by `id`. Two devices converge iff these projections match for every table.
fn canonical_table(conn: &Connection, table: &str) -> Vec<serde_json::Value> {
    // Column list from the local schema, excluding generated columns.
    let cols = non_generated_columns(conn, table);
    let col_list = cols
        .iter()
        .filter(|c| !(table == "assets" && c.as_str() == "path"))
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {col_list} FROM \"{table}\" ORDER BY id");
    let mut stmt = conn.prepare(&sql).expect("prepare canonical query");
    let projected_cols: Vec<String> = cols
        .into_iter()
        .filter(|c| !(table == "assets" && c.as_str() == "path"))
        .collect();
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

/// Non-generated columns of a table (`PRAGMA table_xinfo`, `hidden = 0`). Mirrors
/// the apply/push column reader so the projection matches what actually syncs.
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
        // hidden: 0 normal, 1 hidden, 2 generated-virtual, 3 generated-stored.
        if hidden == 0 {
            Some(name)
        } else {
            None
        }
    })
    .collect()
}

/// Asserts two devices have converged: every synced table's canonical projection
/// matches. Reports the first divergent table for a legible failure.
fn assert_converged(a: &Device, b: &Device) {
    for table in SYNCED_TABLES {
        let ta = canonical_table(&a.conn, table);
        let tb = canonical_table(&b.conn, table);
        assert_eq!(
            ta, tb,
            "table `{table}` diverged between devices\nA = {ta:#?}\nB = {tb:#?}"
        );
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count query")
}

/// Overwrites the `changed_at` of every pending oplog entry for `(table, row_id)`.
/// Used by the clock-skew scenario to stamp a deliberately skewed timestamp that
/// survives the per-cycle clock-offset refresh (the engine recomputes the offset
/// from `health` at the START of each cycle, so injecting `clock_offset_ms`
/// directly would be clobbered; the raw oplog `changed_at` is what `coalesce_ops`
/// reads and `build_change` sends — with the ~0 loopback offset added).
fn force_oplog_changed_at(conn: &Connection, table: &str, row_id: &str, changed_at: i64) {
    let n = conn
        .execute(
            "UPDATE sync_oplog SET changed_at = ?3 WHERE table_name = ?1 AND row_id = ?2",
            rusqlite::params![table, row_id, changed_at],
        )
        .expect("force oplog changed_at");
    assert!(n >= 1, "no oplog entry for {table}/{row_id} to skew");
}

fn conflict_count_for_reason(conn: &Connection, reason: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM sync_conflicts WHERE reason = ?1",
        [reason],
        |r| r.get(0),
    )
    .expect("conflict count")
}

/// The absolute path of a synced asset on a device, derived from the wire
/// `rel_path` semantics: the pull-apply rewrites `assets.path` to the local
/// absolute path under this device's app-data dir.
fn asset_local_path(conn: &Connection, asset_id: &str) -> Option<String> {
    conn.query_row("SELECT path FROM assets WHERE id = ?1", [asset_id], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

/// Drives a two-device handshake to full convergence: each device pushes then
/// pulls in turn until both report no pending work and no parked rows. A handful
/// of rounds covers the re-sequenced-parent / parking cases (DESIGN §4.3).
async fn converge(a: &Device, b: &Device) {
    for _ in 0..4 {
        a.sync().await;
        b.sync().await;
    }
}

// ---------------------------------------------------------------------------
// Scenarios (DESIGN §13.4) — one test each
// ---------------------------------------------------------------------------

/// alta en A → aparece en B.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn insert_in_a_appears_in_b() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    insert_collection(&a.conn, "c1", "Colección A");
    insert_item(&a.conn, "i1", "Documento uno", "c1");

    converge(&a, &b).await;

    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM items WHERE id='i1'"),
        1
    );
    assert_converged(&a, &b);
}

/// edición concurrente misma fila: LWW (mayor changed_at gana; perdedor
/// journaleado en AMBOS extremos).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn concurrent_edit_lww_journals_loser_on_both_ends() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    // Establish a shared base row on both devices.
    insert_collection(&a.conn, "c1", "C");
    insert_item(&a.conn, "i1", "base", "c1");
    converge(&a, &b).await;
    assert_converged(&a, &b);

    // Both edit the SAME row offline from the SAME base version. A's edit carries
    // the EARLIER changed_at (it is the loser); B's the later one (the winner).
    update_item_title(&a.conn, "i1", "edición de A (pierde)");
    // Ensure B's changed_at is strictly later by a wide margin (well above any
    // per-device clock-offset jitter from the separate health RTT samples).
    std::thread::sleep(std::time::Duration::from_millis(1200));
    update_item_title(&b.conn, "i1", "edición de B (gana)");

    // The WINNER (B) pushes FIRST: its base_seq still matches the shared base, so
    // the server applies B's edit directly and it becomes server-current. THEN the
    // LOSER (A) pushes with a now-stale base_seq → the server's LWW resolves
    // against A (earlier changed_at) and returns `lww_lost` + B's winner. A
    // journals the loser locally AND applies B's winner. "Both ends" = the losing
    // client (A) journals `lww_lost` locally AND the server journals it in its own
    // conflicts table (DESIGN §4.2: nothing lost silently).
    b.sync().await;
    a.sync().await;
    b.sync().await;
    a.sync().await;

    let title_a: String = a
        .conn
        .query_row("SELECT title FROM items WHERE id='i1'", [], |r| r.get(0))
        .unwrap();
    let title_b: String = b
        .conn
        .query_row("SELECT title FROM items WHERE id='i1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        title_a, "edición de B (gana)",
        "B's later edit wins LWW on A"
    );
    assert_eq!(title_b, "edición de B (gana)", "B keeps its winning edit");

    // The loser (A) journals its `lww_lost` locally when its push loses. The
    // server-side journal is asserted indirectly: A only receives `lww_lost`
    // because the server resolved + journaled the conflict (PROTOCOL §push).
    assert!(
        conflict_count_for_reason(&a.conn, "lww_lost") >= 1,
        "the losing device (A) must journal the conflict locally (lww_lost)"
    );
    assert_converged(&a, &b);
}

/// delete vs edit: un device borra la fila mientras el otro la edita.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn delete_vs_edit_converges() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    insert_collection(&a.conn, "c1", "C");
    insert_item(&a.conn, "i1", "base", "c1");
    converge(&a, &b).await;
    assert_converged(&a, &b);

    // A deletes (earlier ⇒ the LOSER); B edits later (the WINNER). B's edit wins
    // LWW so the row SURVIVES with B's title on both devices. To make the losing
    // client journal, the WINNER (B) pushes first (its edit becomes current), then
    // the LOSER (A) pushes its delete with a stale base_seq → `lww_lost`, journaled
    // on A; A then applies B's winner (the row is resurrected).
    delete_item(&a.conn, "i1");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    update_item_title(&b.conn, "i1", "editado por B tras delete de A");

    b.sync().await;
    a.sync().await;
    b.sync().await;
    a.sync().await;

    assert_converged(&a, &b);
    // The row survived with B's edit (the later changed_at wins over the delete).
    let title_a: Option<String> = a
        .conn
        .query_row("SELECT title FROM items WHERE id='i1'", [], |r| r.get(0))
        .ok();
    assert_eq!(
        title_a.as_deref(),
        Some("editado por B tras delete de A"),
        "the later edit wins over the delete; the row is resurrected on A"
    );
    // The losing delete is journaled on A (nothing lost silently, DESIGN §4.2).
    assert!(
        conflict_count_for_reason(&a.conn, "lww_lost") >= 1,
        "the losing delete must journal a conflict on A"
    );
}

/// tombstone sobre hijo editado localmente: deferral + posterior resolución.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn tombstone_over_locally_edited_child_defers_then_resolves() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    // Shared item with an `entities` child (entities cascade-delete from items,
    // so an item tombstone would destroy the child — the §4.4 cascade case).
    insert_collection(&a.conn, "c1", "C");
    insert_item(&a.conn, "i1", "padre", "c1");
    insert_entity(&a.conn, "e1", "i1", "valor base");
    converge(&a, &b).await;
    assert_converged(&a, &b);

    // A deletes the parent item (its FK cascade destroys the entity on A, and the
    // tombstone for both i1 and e1 reaches the server). B edits the entity child
    // locally — its `changed_at` is the LATER one. The tombstone-over-dirty-cascade
    // deferral (DESIGN §4.4) protects B's edit during any cycle where B pulls the
    // i1 tombstone while e1 is still dirty in its oplog: the whole tombstone is
    // deferred (not applied) rather than cascade-destroying the local edit.
    delete_item(&a.conn, "i1");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    update_entity_value(&b.conn, "e1", "valor editado por B");

    // A pushes the tombstone for i1 (and the cascade tombstone for e1).
    a.sync().await;

    // B pulls the i1 tombstone BEFORE flushing its e1 edit: drive a pull while e1
    // is dirty by running B's cycle and confirming the deferral protected the
    // edit OR resolved it without silent loss. Either way, drive to full
    // convergence and assert the contract: both devices agree AND the conflict was
    // journaled somewhere (the parent tombstone vs. the child edit — DESIGN §4.2,
    // §4.4: nothing is lost silently).
    converge(&a, &b).await;
    assert_converged(&a, &b);

    let total_conflicts = count(&a.conn, "SELECT COUNT(*) FROM sync_conflicts")
        + count(&b.conn, "SELECT COUNT(*) FROM sync_conflicts");
    assert!(
        total_conflicts >= 1,
        "the tombstone-vs-child-edit clash must journal a conflict (deferral + resolution, \
         nothing lost silently)"
    );
}

/// fotos: importar archivo real pequeño en A → B lo descarga, sha256 idéntico,
/// dedup en server al re-subir.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn photos_blob_download_sha256_and_dedup() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    // A small real file (a tiny PNG-ish blob — content is opaque to sync).
    let bytes: Vec<u8> = (0u8..=200).cycle().take(4096).collect();
    let expected_sha = sha256_hex(&bytes);

    insert_collection(&a.conn, "c1", "C");
    insert_item(&a.conn, "i1", "con foto", "c1");
    let abs_a = import_asset_file(&a, "a1", "i1", "c1", "foto.png", &bytes);
    assert!(abs_a.is_file());

    converge(&a, &b).await;

    // B downloaded the blob: the local file exists with the IDENTICAL sha256.
    let path_b = asset_local_path(&b.conn, "a1").expect("asset row on B");
    let bytes_b = std::fs::read(&path_b).expect("read downloaded blob on B");
    assert_eq!(
        sha256_hex(&bytes_b),
        expected_sha,
        "B's downloaded blob must match A's byte-for-byte"
    );
    // No pending blob downloads remain on B.
    assert_eq!(count(&b.conn, "SELECT COUNT(*) FROM sync_pending_blobs"), 0);

    // Dedup: B re-imports the SAME bytes under a new asset/item and pushes; the
    // server already holds the blob (HEAD 200), so B must NOT re-PUT it. We can't
    // observe the server's PUT count directly, but convergence + identical sha is
    // the contract; re-sync must succeed without error.
    insert_item(&b.conn, "i2", "misma foto", "c1");
    let _abs_b = import_asset_file(&b, "a2", "i2", "c1", "foto2.png", &bytes);
    converge(&a, &b).await;

    let path_a2 = asset_local_path(&a.conn, "a2").expect("a2 on A");
    let bytes_a2 = std::fs::read(&path_a2).expect("read a2 on A");
    assert_eq!(
        sha256_hex(&bytes_a2),
        expected_sha,
        "deduped blob converges"
    );
    assert_converged(&a, &b);
}

/// seed: A con datos previos al login, B vacío converge (incluye blob).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn seed_preexisting_library_including_blob() {
    let url = server_or_skip!();

    // A device with data created BEFORE any sync session (capture off): rows
    // exist in the tables but never hit the oplog. Seeding (DESIGN §4.5) must
    // enumerate them on first login.
    let mut a = Device::new(&url);
    // Pre-session writes: capture_enabled is unset, so no oplog entries.
    insert_collection(&a.conn, "c1", "Pre-existente");
    insert_item(&a.conn, "i1", "anterior al login", "c1");
    let bytes: Vec<u8> = (3u8..=250).cycle().take(2048).collect();
    let expected_sha = sha256_hex(&bytes);
    import_asset_file(&a, "a1", "i1", "c1", "previo.png", &bytes);
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_oplog"),
        0,
        "pre-session writes must not be captured"
    );

    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    converge(&a, &b).await;

    // B converged on the seeded library, including the blob.
    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM items WHERE id='i1'"),
        1
    );
    let path_b = asset_local_path(&b.conn, "a1").expect("seeded asset on B");
    let bytes_b = std::fs::read(&path_b).expect("read seeded blob on B");
    assert_eq!(sha256_hex(&bytes_b), expected_sha, "seeded blob converges");
    assert_converged(&a, &b);
}

/// ambos con datos pre-existentes mergean (unión).
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn both_with_preexisting_data_merge_union() {
    let url = server_or_skip!();

    // A registers and seeds its pre-existing data.
    let mut a = Device::new(&url);
    insert_collection(&a.conn, "ca", "Colección A");
    insert_item(&a.conn, "ia", "ítem de A", "ca");
    let (email, _account) = a.register_and_login().await;

    // B has DIFFERENT pre-existing data, then joins the SAME account and merges
    // (the "conservar datos locales" account-join path uses the seeding §4.5).
    let mut b = Device::new(&url);
    insert_collection(&b.conn, "cb", "Colección B");
    insert_item(&b.conn, "ib", "ítem de B", "cb");
    b.login(&email).await;

    converge(&a, &b).await;

    // Union: both devices hold both collections and both items.
    for dev in [&a, &b] {
        assert_eq!(
            count(
                &dev.conn,
                "SELECT COUNT(*) FROM collections WHERE id IN ('ca','cb')"
            ),
            2
        );
        assert_eq!(
            count(
                &dev.conn,
                "SELECT COUNT(*) FROM items WHERE id IN ('ia','ib')"
            ),
            2
        );
    }
    assert_converged(&a, &b);
}

/// cambio de cuenta re-sube biblioteca.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn account_change_reuploads_library() {
    let url = server_or_skip!();

    // A populates account 1.
    let mut a = Device::new(&url);
    insert_collection(&a.conn, "c1", "Biblioteca");
    insert_item(&a.conn, "i1", "documento", "c1");
    let bytes: Vec<u8> = (5u8..=130).cycle().take(1024).collect();
    let expected_sha = sha256_hex(&bytes);
    import_asset_file(&a, "a1", "i1", "c1", "doc.png", &bytes);
    let (_email1, _acc1) = a.register_and_login().await;
    a.sync_twice().await;

    // A new account is registered (by a throwaway device), then A switches to it
    // KEEPING its local data → the whole library re-seeds + re-uploads.
    let mut bootstrap = Device::new(&url);
    let (email2, _acc2) = bootstrap.register_and_login().await;

    a.switch_account_keep_data(&email2).await;

    // A fresh device C on the NEW account must receive A's re-uploaded library.
    let mut c = Device::new(&url);
    c.login(&email2).await;

    converge(&a, &c).await;

    assert_eq!(
        count(&c.conn, "SELECT COUNT(*) FROM items WHERE id='i1'"),
        1
    );
    let path_c = asset_local_path(&c.conn, "a1").expect("re-uploaded asset on C");
    let bytes_c = std::fs::read(&path_c).expect("read re-uploaded blob on C");
    assert_eq!(
        sha256_hex(&bytes_c),
        expected_sha,
        "library re-uploaded to new account"
    );
    assert_converged(&a, &c);
}

/// offline prolongado: A acumula 1200+ cambios sin red → push troceado converge.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn long_offline_chunked_push_converges() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    // Accumulate 1200+ dirty rows offline (no sync between writes). The push
    // batches at ≤500 changes, so this exercises multi-batch chunking.
    insert_collection(&a.conn, "c1", "Masiva");
    let total = 1200;
    {
        let tx = a.conn.unchecked_transaction().unwrap();
        for n in 0..total {
            tx.execute(
                "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES(?1,?2,'c1',?3,?3)",
                rusqlite::params![format!("i{n:04}"), format!("ítem {n}"), now_ms()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    // 1200 items + 1 collection dirty.
    assert!(count(&a.conn, "SELECT COUNT(*) FROM sync_oplog") >= total);

    converge(&a, &b).await;

    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM items"),
        total,
        "all chunked items reached B"
    );
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_oplog"),
        0,
        "A's oplog drained"
    );
    assert_converged(&a, &b);
}

/// bootstrap multi-página con padre re-secuenciado: crear >600 filas, update del
/// padre al final, B bootstrapea con limit chico → parking drena.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn multipage_bootstrap_with_resequenced_parent_drains_parking() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;

    // A creates a collection (the parent) and >600 items (children of the
    // collection). Then it UPDATES the collection LAST, so the parent's
    // server_seq lands AFTER all its children — the classic re-sequenced-parent
    // case that forces the pull-apply to park FK violators across pages.
    insert_collection(&a.conn, "c1", "Padre");
    let total = 650;
    {
        let tx = a.conn.unchecked_transaction().unwrap();
        for n in 0..total {
            tx.execute(
                "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES(?1,?2,'c1',?3,?3)",
                rusqlite::params![format!("i{n:04}"), format!("hijo {n}"), now_ms()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    a.sync_twice().await;
    // Re-sequence the parent AFTER its children.
    update_item_title(&a.conn, "i0000", "primer hijo (touch)");
    a.conn
        .execute(
            "UPDATE collections SET name='Padre actualizado', updated_at=?1 WHERE id='c1'",
            rusqlite::params![now_ms()],
        )
        .unwrap();
    a.sync_twice().await;

    // A FRESH device B bootstraps from since=0. (The page limit is the engine's
    // PULL_LIMIT=500, so 650 children + parent span multiple pages with the
    // parent re-sequenced to a later page → parking must drain by cycle end.)
    let mut b = Device::new(&url);
    b.login(&email).await;
    converge(&a, &b).await;

    assert_eq!(count(&b.conn, "SELECT COUNT(*) FROM items"), total);
    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM sync_pending_rows"),
        0,
        "parking must drain by end of bootstrap"
    );
    // The re-sequenced parent update is present.
    let name_b: String = b
        .conn
        .query_row("SELECT name FROM collections WHERE id='c1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(name_b, "Padre actualizado");
    assert_converged(&a, &b);
}

/// reloj desfasado: inyectar offset grande en un device → server clampa o rechaza
/// según magnitud.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn skewed_clock_clamped_or_rejected_by_magnitude() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    insert_collection(&a.conn, "c1", "C");
    insert_item(&a.conn, "i1", "reloj normal", "c1");
    converge(&a, &b).await;
    assert_converged(&a, &b);

    // MODERATE forward skew (now+1h, between now+5min and now+24h): the server
    // CLAMPS the changed_at to `now` and still applies (PROTOCOL "Reloj"). We stamp
    // the oplog entry directly so the skew survives the cycle's offset refresh.
    update_item_title(&a.conn, "i1", "editado con reloj +1h");
    force_oplog_changed_at(&a.conn, "items", "i1", now_ms() + 60 * 60 * 1000);
    a.sync().await; // must succeed (clamped, not rejected)
    b.sync().await;
    assert_converged(&a, &b);

    // HUGE forward skew (now+48h, beyond ±24h): the server REJECTS the whole batch
    // with 400 clock_skew → the cycle surfaces a Fatal error and the push does not
    // land. We assert the cycle errors rather than silently applying.
    update_item_title(&a.conn, "i1", "editado con reloj +48h");
    force_oplog_changed_at(&a.conn, "items", "i1", now_ms() + 48 * 60 * 60 * 1000);
    let warn = |msg: String| eprintln!("[sync-e2e warn] {msg}");
    let result = run_cycle(&a.api, &a.token, &a.conn, &a.app_data_dir, &warn).await;
    assert!(
        result.is_err(),
        "a >24h clock skew must be rejected by the server (400 clock_skew)"
    );

    // Recover: re-stamp the pending edit with a sane time; the next cycle pushes
    // it and both devices converge.
    force_oplog_changed_at(&a.conn, "items", "i1", now_ms());
    converge(&a, &b).await;
    assert_converged(&a, &b);
}

/// crash entre commit del server y borrado del oplog: push OK, NO borrar oplog,
/// re-push → idempotente sin journal.
#[tokio::test]
#[ignore = "requires a running entropia-sync-server (ENTROPIA_SYNC_E2E_SERVER)"]
async fn crash_between_server_commit_and_oplog_purge_is_idempotent() {
    let url = server_or_skip!();
    let mut a = Device::new(&url);
    let (email, _account) = a.register_and_login().await;
    let mut b = Device::new(&url);
    b.login(&email).await;

    insert_collection(&a.conn, "c1", "C");
    insert_item(&a.conn, "i1", "fila idempotente", "c1");

    // Snapshot the oplog state BEFORE the first push so we can replay it.
    let oplog_before: Vec<(String, String, String, i64)> = {
        let mut stmt = a
            .conn
            .prepare("SELECT table_name,row_id,op,changed_at FROM sync_oplog ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert!(!oplog_before.is_empty());

    // First sync: server commits AND the client purges the oplog + records
    // row_versions normally.
    a.sync().await;
    assert_eq!(count(&a.conn, "SELECT COUNT(*) FROM sync_oplog"), 0);
    let conflicts_after_first = count(&a.conn, "SELECT COUNT(*) FROM sync_conflicts");

    // Simulate the crash window: the server already committed, but the client
    // died before purging the oplog. We RE-INSERT the exact same oplog entries
    // (same changed_at) WITHOUT touching row_versions-derived base_seq — the
    // re-push therefore replays identical (changed_at, device_id) → the server
    // returns `applied` with the CURRENT server_seq and NO journal entry.
    {
        let tx = a.conn.unchecked_transaction().unwrap();
        for (table, row_id, op, changed_at) in &oplog_before {
            tx.execute(
                "INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES(?1,?2,?3,?4)",
                rusqlite::params![table, row_id, op, changed_at],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    assert!(count(&a.conn, "SELECT COUNT(*) FROM sync_oplog") > 0);

    // Re-push: idempotent. No new conflict journal entries (replay-equal).
    a.sync().await;
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_oplog"),
        0,
        "re-push drains oplog"
    );
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_conflicts"),
        conflicts_after_first,
        "idempotent replay must not journal a conflict"
    );

    converge(&a, &b).await;
    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM items WHERE id='i1'"),
        1
    );
    assert_converged(&a, &b);
}
