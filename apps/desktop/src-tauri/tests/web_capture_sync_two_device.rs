//! Two-device acceptance for saved web captures (`web_sources`, `web_captures`,
//! capability `web-capture-v1`).
//!
//! Two independent devices, each with its OWN temp `app_data_dir` and its OWN
//! SQLite DB built from `tests/fixtures/schema_full.sql`, run the public
//! `sync::engine::run_cycle` over the real `sync::http::HttpSyncApi` against a
//! real EntropIA-Cloud server binary on loopback. No mock transport, no
//! production database, no production server: the session token is held in
//! memory and never touches the OS keyring.
//!
//! Server resolution, in order (same contract as the sibling two-device suites):
//!   1. `ENTROPIA_SYNC_E2E_SERVER` - a server already running.
//!   2. `ENTROPIA_SYNC_SERVER_BIN` - explicit path to `entropia-sync-server`.
//!   3. The sibling checkout `EntropIA-Cloud/target`.
//!
//! With none available the tests print a loud skip notice and return. A
//! reachable server that does not advertise `web-capture-v1` FAILS loudly: a
//! legacy server can never be an acceptance target for this suite.
//!
//! Build the server with the web-capture capability from the Cloud checkout that
//! carries it (`cargo build` there) and point `ENTROPIA_SYNC_SERVER_BIN` at
//! `<cloud>/target/debug/entropia-sync-server(.exe)`.
//!
//! Save-side fixture policy: the Tauri save command is unreachable from an
//! integration crate, so captures are written the way `navegador::save` writes
//! them (file at `web-captures/<source>/<capture>.<ext>`, row with the real
//! digest); the sync code under test is entirely production code.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use entropia_desktop_lib::sync::capture::ensure_capture;
use entropia_desktop_lib::sync::engine::{read_schema_tag, run_cycle};
use entropia_desktop_lib::sync::http::{HttpSyncApi, LoginRequest, RegisterRequest, SyncApi};
use entropia_desktop_lib::sync::session::meta_set;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");
const SERVER_ENV: &str = "ENTROPIA_SYNC_E2E_SERVER";
const SERVER_BIN_ENV: &str = "ENTROPIA_SYNC_SERVER_BIN";
const PASSWORD: &str = "web-capture-password-123";

static EMAIL_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_email() -> String {
    let n = EMAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("web-{}-{n}@entropia.test", std::process::id())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
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
// Test server
// ---------------------------------------------------------------------------

struct TestServer {
    base_url: String,
    _data_dir: Option<tempfile::TempDir>,
    child: Option<Child>,
}

impl TestServer {
    async fn start() -> Option<TestServer> {
        if let Ok(url) = std::env::var(SERVER_ENV) {
            let trimmed = url.trim().trim_end_matches('/').to_string();
            if !trimmed.is_empty() {
                return Some(TestServer {
                    base_url: trimmed,
                    _data_dir: None,
                    child: None,
                });
            }
        }
        let bin = find_server_bin()?;
        let data_dir = tempfile::tempdir().expect("server data dir");
        // A port freed by `free_port` can still be refused to the server on
        // Windows (another socket may hold it exclusively): the child then exits
        // at once with "address in use". Retry on a fresh port instead of
        // failing the whole suite on one busy port.
        let mut attempt = 0;
        let (child, base_url) = loop {
            attempt += 1;
            let bind = format!("127.0.0.1:{}", free_port());
            let mut child = Command::new(&bin)
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
            let exited = loop {
                if api.health().await.is_ok() {
                    break false;
                }
                if child
                    .try_wait()
                    .expect("poll entropia-sync-server")
                    .is_some()
                {
                    break true;
                }
                assert!(
                    Instant::now() < deadline,
                    "entropia-sync-server did not answer /v1/health at {base_url} within 30s"
                );
                tokio::time::sleep(Duration::from_millis(250)).await;
            };
            if !exited {
                break (child, base_url);
            }
            assert!(
                attempt < 5,
                "entropia-sync-server exited at start on 5 fresh ports"
            );
        };
        Some(TestServer {
            base_url,
            _data_dir: Some(data_dir),
            child: Some(child),
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

fn find_server_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(SERVER_BIN_ENV) {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../EntropIA-Cloud/target");
    let name = if cfg!(windows) {
        "entropia-sync-server.exe"
    } else {
        "entropia-sync-server"
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for dir in ["debug", "release"] {
        let candidate = root.join(dir).join(name);
        if let Ok(modified) = std::fs::metadata(&candidate).and_then(|m| m.modified()) {
            if best.as_ref().is_none_or(|(at, _)| modified > *at) {
                best = Some((modified, candidate));
            }
        }
    }
    best.map(|(_, path)| path)
}

fn skip_notice() {
    eprintln!(
        "[web-capture-sync-two-device] SKIP: no test server available. Set {SERVER_ENV}=<base-url> \
         or {SERVER_BIN_ENV}=<path to a built entropia-sync-server that advertises web-capture-v1>."
    );
}

// ---------------------------------------------------------------------------
// Device harness
// ---------------------------------------------------------------------------

struct Device {
    _tempdir: tempfile::TempDir,
    app_data_dir: PathBuf,
    conn: Connection,
    api: HttpSyncApi,
    server_url: String,
    token: String,
}

impl Device {
    fn new(server_url: &str) -> Device {
        let tempdir = tempfile::tempdir().expect("device temp dir");
        let app_data_dir = tempdir.path().to_path_buf();
        std::fs::create_dir_all(app_data_dir.join("assets")).expect("assets dir");
        let conn = open_conn(&app_data_dir.join("entropia.sqlite"));
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("application schema");
        conn.execute(
            "INSERT INTO _migrations(name, applied_at) VALUES (?1, ?2)",
            params!["0057_web_captures", now_ms()],
        )
        .expect("migration head");
        ensure_capture(&conn).expect("sync schema + capture triggers");
        Device {
            _tempdir: tempdir,
            app_data_dir,
            conn,
            api: HttpSyncApi::new(server_url).expect("http sync api"),
            server_url: server_url.to_string(),
            token: String::new(),
        }
    }

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

    async fn login(&mut self, email: &str) {
        let resp = self
            .api
            .login(LoginRequest {
                email: email.to_string(),
                password: PASSWORD.to_string(),
                device_name: format!("web-{}", std::process::id()),
                platform: "test".to_string(),
            })
            .await
            .expect("login");
        self.token = resp.device_token;
        meta_set(&self.conn, "server_url", &self.server_url).unwrap();
        meta_set(&self.conn, "account_id", &resp.account_id).unwrap();
        meta_set(&self.conn, "account_email", email).unwrap();
        meta_set(&self.conn, "device_id", &resp.device_id).unwrap();
        meta_set(&self.conn, "capture_enabled", "1").unwrap();
        meta_set(
            &self.conn,
            "sync_session_incarnation",
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
    }

    async fn assert_web_capture_capability(&self) {
        let schema_tag = read_schema_tag(&self.conn).expect("schema tag");
        let response = self
            .api
            .pull(&self.token, &schema_tag, 0, 1)
            .await
            .expect("capability probe pull");
        assert!(
            response.capabilities.iter().any(|c| c == "web-capture-v1"),
            "the test server does not advertise web-capture-v1; rebuild the EntropIA-Cloud \
             server from the web-capture branch or point {SERVER_ENV} at a current server"
        );
    }

    async fn sync(&self) {
        let quiet = |_message: String| {};
        run_cycle(
            &self.api,
            &self.token,
            &self.conn,
            &self.app_data_dir,
            &quiet,
        )
        .await
        .unwrap_or_else(|e| panic!("sync cycle failed: {e:?}"));
    }

    fn file_path(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.app_data_dir.clone(), |acc, part| acc.join(part))
    }

    fn source_dir(&self, source_id: &str) -> PathBuf {
        self.app_data_dir.join("web-captures").join(source_id)
    }

    fn ensure_source(&self, source_id: &str, url: &str) {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO web_sources
                   (id, original_url, final_url, title, first_accessed_at, created_at, updated_at)
                 VALUES (?1, ?2, ?2, 'A saved page', '2026-10-02T00:00:00Z', ?3, ?3)",
                params![source_id, url, now_ms()],
            )
            .expect("insert source");
    }

    /// A page capture: HTML snapshot file, plus a long-text file when given.
    fn save_page(&self, source_id: &str, capture_id: &str, html: &[u8], long_text: Option<&[u8]>) {
        self.ensure_source(source_id, "https://example.org/article");
        let rel = format!("web-captures/{source_id}/{capture_id}.html");
        self.write_file(&rel, html);
        let text_rel = long_text.map(|text| {
            let rel = format!("web-captures/{source_id}/{capture_id}.txt");
            self.write_file(&rel, text);
            rel
        });
        self.conn
            .execute(
                "INSERT INTO web_captures
                   (id, web_source_id, accessed_at, final_url, kind, mime_type, text,
                    text_rel_path, rel_path, sha256, hash_of, size_bytes, extractor_version,
                    title, created_at)
                 VALUES (?1, ?2, '2026-10-02T00:00:00Z', 'https://example.org/article', 'page',
                         'text/html', NULL, ?3, ?4, ?5, 'html', ?6, 'navegador-capture-1',
                         'A saved page', ?7)",
                params![
                    capture_id,
                    source_id,
                    text_rel,
                    rel,
                    sha256_hex(html),
                    html.len() as i64,
                    now_ms()
                ],
            )
            .expect("insert page capture");
    }

    /// A selection capture: the quote lives in the row, no file.
    fn save_selection(&self, source_id: &str, capture_id: &str, quote: &str) {
        self.ensure_source(source_id, "https://example.org/article");
        self.conn
            .execute(
                "INSERT INTO web_captures
                   (id, web_source_id, accessed_at, final_url, kind, mime_type, text,
                    quote_prefix, quote_suffix, sha256, hash_of, size_bytes, created_at)
                 VALUES (?1, ?2, '2026-10-02T00:01:00Z', 'https://example.org/article',
                         'selection', 'text/plain', ?3, 'before ', ' after', ?4, 'quote', ?5, ?6)",
                params![
                    capture_id,
                    source_id,
                    quote,
                    sha256_hex(quote.as_bytes()),
                    quote.len() as i64,
                    now_ms()
                ],
            )
            .expect("insert selection capture");
    }

    /// A PDF capture: the PDF file plus its row.
    fn save_pdf(&self, source_id: &str, capture_id: &str, pdf: &[u8]) {
        self.ensure_source(source_id, "https://example.org/paper.pdf");
        let rel = format!("web-captures/{source_id}/{capture_id}.pdf");
        self.write_file(&rel, pdf);
        self.conn
            .execute(
                "INSERT INTO web_captures
                   (id, web_source_id, accessed_at, final_url, kind, mime_type, rel_path,
                    sha256, hash_of, size_bytes, created_at)
                 VALUES (?1, ?2, '2026-10-02T00:02:00Z', 'https://example.org/paper.pdf', 'pdf',
                         'application/pdf', ?3, ?4, 'pdf', ?5, ?6)",
                params![
                    capture_id,
                    source_id,
                    rel,
                    sha256_hex(pdf),
                    pdf.len() as i64,
                    now_ms()
                ],
            )
            .expect("insert pdf capture");
    }

    fn write_file(&self, rel: &str, bytes: &[u8]) {
        let path = self.file_path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).expect("capture dir");
        std::fs::write(path, bytes).expect("write capture file");
    }

    /// What `navegador::sources::delete_source` does: captures first, then the
    /// source row, then the folder.
    fn delete_source(&self, source_id: &str) {
        self.conn
            .execute(
                "DELETE FROM web_captures WHERE web_source_id = ?1",
                [source_id],
            )
            .expect("delete captures");
        self.conn
            .execute("DELETE FROM web_sources WHERE id = ?1", [source_id])
            .expect("delete source");
        let _ = std::fs::remove_dir_all(self.source_dir(source_id));
    }

    /// A corpus copy of a capture, as "Copiar a coleccion" creates it. Sync of
    /// web sources must never touch it.
    fn add_copy(&self, item_id: &str, source_id: &str, capture_id: &str) {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO collections(id, name, created_at, updated_at)
                 VALUES ('col-1', 'Corpus', 1, 1)",
                [],
            )
            .expect("collection");
        let metadata = serde_json::json!({
            "__entropia_web_capture": { "source_id": source_id, "capture_id": capture_id }
        })
        .to_string();
        self.conn
            .execute(
                "INSERT INTO items(id, title, collection_id, metadata, created_at, updated_at)
                 VALUES (?1, 'A copy', 'col-1', ?2, 1, 1)",
                params![item_id, metadata],
            )
            .expect("copy item");
    }

    fn file_digest(&self, rel: &str) -> Option<String> {
        std::fs::read(self.file_path(rel))
            .ok()
            .map(|bytes| sha256_hex(&bytes))
    }
}

fn pdf_bytes(len: usize) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut state: u32 = 0x9e37_79b9;
    while bytes.len() < len {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        bytes.push((state >> 24) as u8);
    }
    bytes.extend_from_slice(b"\n%%EOF\n");
    bytes
}

const SOURCE_A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const SOURCE_B: &str = "bbbbbbbb-0000-4000-8000-00000000000b";
const PAGE: &str = "cccccccc-0000-4000-8000-00000000000c";
const SELECTION: &str = "dddddddd-0000-4000-8000-00000000000d";
const PDF: &str = "eeeeeeee-0000-4000-8000-00000000000e";

/// Two devices on one account, A seeded with a page, a selection and a PDF.
async fn two_devices(server: &TestServer) -> (Device, Device, Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut a = Device::new(&server.base_url);
    let email = a.register_and_login().await;
    a.assert_web_capture_capability().await;
    let html = b"<html><body><h1>Saved page</h1></body></html>".to_vec();
    let long_text = "a long readable text ".repeat(40_000).into_bytes(); // ~840 KB
    let pdf = pdf_bytes(3 * 1024 * 1024 + 123); // streamed upload, > one chunk
    a.save_page(SOURCE_A, PAGE, &html, Some(&long_text));
    a.save_selection(SOURCE_A, SELECTION, "the exact quote");
    a.save_pdf(SOURCE_B, PDF, &pdf);
    a.add_copy("item-copy", SOURCE_B, PDF);

    let mut b = Device::new(&server.base_url);
    b.login(&email).await;
    (a, b, html, long_text, pdf)
}

#[tokio::test]
async fn a_page_a_selection_and_a_pdf_reach_the_other_device_byte_identical() {
    let Some(server) = TestServer::start().await else {
        return skip_notice();
    };
    let (a, b, html, long_text, pdf) = two_devices(&server).await;

    a.sync().await;
    b.sync().await;
    // A second round settles anything that needed a later cycle.
    a.sync().await;
    b.sync().await;

    assert_eq!(count(&b.conn, "SELECT COUNT(*) FROM web_sources"), 2);
    assert_eq!(count(&b.conn, "SELECT COUNT(*) FROM web_captures"), 3);
    for (table, ids) in [
        ("web_sources", vec![SOURCE_A, SOURCE_B]),
        ("web_captures", vec![PAGE, SELECTION, PDF]),
    ] {
        for id in ids {
            let same: i64 = count(
                &b.conn,
                &format!("SELECT COUNT(*) FROM {table} WHERE id = '{id}'"),
            );
            assert_eq!(same, 1, "{table} {id} missing on B");
        }
    }

    // Row content survives the trip (spot-check the immutable evidence fields).
    let (sha, hash_of, size): (String, String, i64) = b
        .conn
        .query_row(
            "SELECT sha256, hash_of, size_bytes FROM web_captures WHERE id = ?1",
            [PAGE],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(sha, sha256_hex(&html));
    assert_eq!(hash_of, "html");
    assert_eq!(size, html.len() as i64);
    let quote: String = b
        .conn
        .query_row(
            "SELECT text FROM web_captures WHERE id = ?1",
            [SELECTION],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(quote, "the exact quote");

    // Files: same relative key, byte-identical.
    for (rel, bytes) in [
        (format!("web-captures/{SOURCE_A}/{PAGE}.html"), &html),
        (format!("web-captures/{SOURCE_A}/{PAGE}.txt"), &long_text),
        (format!("web-captures/{SOURCE_B}/{PDF}.pdf"), &pdf),
    ] {
        assert_eq!(
            b.file_digest(&rel).as_deref(),
            Some(sha256_hex(bytes).as_str()),
            "{rel} on B differs from A"
        );
    }
    assert_eq!(
        count(&b.conn, "SELECT COUNT(*) FROM sync_web_pending_blobs"),
        0,
        "nothing left to download"
    );
    assert_eq!(
        count(
            &a.conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name LIKE 'web_%'"
        ),
        0,
        "A has nothing left to push"
    );
    assert_eq!(
        count(
            &b.conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name LIKE 'web_%'"
        ),
        0,
        "B did not echo the rows back"
    );
}

#[tokio::test]
async fn deleting_a_source_on_one_device_removes_it_and_its_folder_on_the_other() {
    let Some(server) = TestServer::start().await else {
        return skip_notice();
    };
    let (a, b, ..) = two_devices(&server).await;
    // A copy of the PDF capture exists on both devices' corpus.
    b.add_copy("item-copy", SOURCE_B, PDF);
    a.sync().await;
    b.sync().await;
    assert!(b.source_dir(SOURCE_B).exists(), "B holds the PDF folder");

    b.delete_source(SOURCE_B);
    b.sync().await;
    a.sync().await;
    a.sync().await;

    assert_eq!(
        count(
            &a.conn,
            &format!("SELECT COUNT(*) FROM web_sources WHERE id='{SOURCE_B}'")
        ),
        0
    );
    assert_eq!(
        count(
            &a.conn,
            &format!("SELECT COUNT(*) FROM web_captures WHERE id='{PDF}'")
        ),
        0
    );
    assert!(
        !a.source_dir(SOURCE_B).exists(),
        "A's folder followed the rows"
    );
    // The other source is untouched on both sides.
    assert!(a.source_dir(SOURCE_A).join(format!("{PAGE}.html")).exists());
    assert_eq!(
        count(
            &a.conn,
            &format!("SELECT COUNT(*) FROM web_captures WHERE id='{PAGE}'")
        ),
        1
    );
    // Copies are independent: the corpus item survives on both devices.
    for device in [&a, &b] {
        assert_eq!(
            count(
                &device.conn,
                "SELECT COUNT(*) FROM items WHERE id='item-copy'"
            ),
            1,
            "the corpus copy is untouched"
        );
    }
}

#[tokio::test]
async fn a_device_whose_cursor_passed_the_rows_catches_up_once() {
    let Some(server) = TestServer::start().await else {
        return skip_notice();
    };
    let (a, b, html, ..) = two_devices(&server).await;
    a.sync().await;

    // B behaves like a client updated from a legacy version: its shared cursor
    // already sits at the head of the account, so the web rows are "behind" it.
    let schema_tag = read_schema_tag(&b.conn).unwrap();
    let head = b
        .api
        .pull(&b.token, &schema_tag, 0, 1000)
        .await
        .expect("probe")
        .next_since;
    meta_set(&b.conn, "last_pull_seq", &head.to_string()).unwrap();
    let account_id: String = b
        .conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key='account_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    meta_set(&b.conn, "seeded_account", &account_id).unwrap();

    b.sync().await;
    assert_eq!(count(&b.conn, "SELECT COUNT(*) FROM web_captures"), 3);
    assert_eq!(
        b.file_digest(&format!("web-captures/{SOURCE_A}/{PAGE}.html"))
            .as_deref(),
        Some(sha256_hex(&html).as_str())
    );
}

#[tokio::test]
async fn a_new_capture_on_a_synced_device_arrives_incrementally() {
    let Some(server) = TestServer::start().await else {
        return skip_notice();
    };
    let (a, b, ..) = two_devices(&server).await;
    a.sync().await;
    b.sync().await;

    let html = b"<html>second capture of the same page</html>";
    a.save_page(SOURCE_A, "ffffffff-0000-4000-8000-00000000000f", html, None);
    a.sync().await;
    b.sync().await;
    assert_eq!(count(&b.conn, "SELECT COUNT(*) FROM web_captures"), 4);
    assert_eq!(
        b.file_digest(&format!(
            "web-captures/{SOURCE_A}/ffffffff-0000-4000-8000-00000000000f.html"
        ))
        .as_deref(),
        Some(sha256_hex(html).as_str())
    );
}
