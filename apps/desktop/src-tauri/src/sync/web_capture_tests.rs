//! Tests for the `web-capture-v1` row sync: capability record, push gating,
//! the one-time catch-up, and apply of the two web tables.

use serde_json::json;

use super::apply::{apply_page, retry_pending_rows, ApplyContext};
use super::engine::run_cycle;
use super::http::{PullResponse, PullRow, WEB_CAPTURE_V1_CAPABILITY as WEB_CAPTURE_CAPABILITY};
use super::session::{meta_get_i64, meta_set};
use super::test_support::{new_synced_test_db, set_session_with_capture, MockSyncApi};
use super::web_capture::{
    catchup_needed, clear_account_metadata, record_capability, supports_web_capture,
};
use rusqlite::Connection;

const ACCOUNT: &str = "mock-account";
const EPOCH: &str = "mock-epoch";

fn session_db() -> Connection {
    let conn = new_synced_test_db();
    super::capture::ensure_capture(&conn).expect("ensure capture");
    set_session_with_capture(&conn);
    meta_set(&conn, "account_id", ACCOUNT).unwrap();
    meta_set(&conn, "server_url", "https://sync.test").unwrap();
    meta_set(&conn, "seeded_account", ACCOUNT).unwrap();
    conn
}

fn app_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("assets")).expect("assets dir");
    dir
}

fn insert_source(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO web_sources(id, original_url, final_url, first_accessed_at, created_at, updated_at)
         VALUES (?1, 'https://a.test/', 'https://a.test/', '2026-10-02T00:00:00Z', 1, 1)",
        [id],
    )
    .expect("insert source");
}

fn insert_capture(conn: &Connection, id: &str, source: &str) {
    conn.execute(
        "INSERT INTO web_captures(id, web_source_id, accessed_at, final_url, kind, mime_type,
                                  text, sha256, hash_of, size_bytes, created_at)
         VALUES (?1, ?2, '2026-10-02T00:00:00Z', 'https://a.test/', 'selection', 'text/plain',
                 'quote', 'ab', 'quote', 5, 1)",
        [id, source],
    )
    .expect("insert capture");
}

fn source_row(id: &str, seq: i64) -> PullRow {
    PullRow {
        table: "web_sources".to_string(),
        row_id: id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: Some(json!({
            "id": id, "original_url": "https://r.test/", "final_url": "https://r.test/",
            "canonical_url": null, "title": "Remote", "site_name": null,
            "first_accessed_at": "2026-10-02T00:00:00Z", "created_at": 1, "updated_at": 1
        })),
    }
}

fn capture_row(id: &str, source: &str, seq: i64) -> PullRow {
    PullRow {
        table: "web_captures".to_string(),
        row_id: id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: Some(json!({
            "id": id, "web_source_id": source, "accessed_at": "2026-10-02T00:00:00Z",
            "final_url": "https://r.test/", "kind": "selection", "mime_type": "text/plain",
            "text": "quote", "text_rel_path": null, "quote_prefix": null, "quote_suffix": null,
            "rel_path": null, "sha256": "ab", "hash_of": "quote", "size_bytes": 5,
            "extractor_version": null, "title": null, "created_at": 1
        })),
    }
}

fn tombstone(table: &str, id: &str, seq: i64) -> PullRow {
    PullRow {
        table: table.to_string(),
        row_id: id.to_string(),
        server_seq: seq,
        deleted: true,
        changed_at: 2,
        device_id: "remote".to_string(),
        payload: None,
    }
}

fn page(rows: Vec<PullRow>, next_since: i64, caps: &[&str]) -> PullResponse {
    PullResponse {
        rows,
        next_since,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: caps.iter().map(|c| c.to_string()).collect(),
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn capability_token_is_the_protocol_token() {
    assert_eq!(WEB_CAPTURE_CAPABILITY, "web-capture-v1");
}

#[test]
fn capability_is_trusted_only_for_the_epoch_that_advertised_it() {
    let conn = new_synced_test_db();
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    assert!(!supports_web_capture(&conn).unwrap(), "nothing recorded");

    record_capability(&conn, EPOCH, true).unwrap();
    assert!(supports_web_capture(&conn).unwrap());

    // A restored/rotated server (new epoch) must re-advertise.
    meta_set(&conn, "server_epoch", "other-epoch").unwrap();
    assert!(!supports_web_capture(&conn).unwrap());

    // A later ordinary response that omits the token (rolled-back server).
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    record_capability(&conn, EPOCH, false).unwrap();
    assert!(!supports_web_capture(&conn).unwrap());
}

#[test]
fn clearing_account_metadata_forgets_capability_and_catchup() {
    let conn = new_synced_test_db();
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    record_capability(&conn, EPOCH, true).unwrap();
    super::web_capture::record_catchup_done(&conn, EPOCH).unwrap();
    assert!(!catchup_needed(&conn, EPOCH).unwrap());

    clear_account_metadata(&conn).unwrap();
    assert!(!supports_web_capture(&conn).unwrap());
    assert!(catchup_needed(&conn, EPOCH).unwrap());
}

#[tokio::test]
async fn web_rows_stay_in_the_oplog_until_the_server_advertises_the_capability() {
    let conn = session_db();
    insert_source(&conn, "s1");
    insert_capture(&conn, "c1", "s1");
    let dir = app_dir();

    // A legacy server: no capability. The rows must not be sent (an old server
    // would reject the whole batch) and must stay pending.
    let legacy = MockSyncApi::default();
    run_cycle(&legacy, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    assert_eq!(
        legacy.pushed_count(),
        0,
        "no web row reaches a legacy server"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name LIKE 'web_%'"
        ),
        2,
        "web rows are held, not dropped"
    );

    // The server is upgraded: the same cycle discovers the token on its pull
    // response and sends the held rows.
    let modern = MockSyncApi::default();
    modern
        .server_capabilities
        .lock()
        .unwrap()
        .push(WEB_CAPTURE_CAPABILITY.to_string());
    run_cycle(&modern, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    let pushed: Vec<String> = modern
        .pushed
        .lock()
        .unwrap()
        .iter()
        .map(|c| format!("{}:{}", c.table, c.row_id))
        .collect();
    assert_eq!(pushed.len(), 2, "both rows pushed: {pushed:?}");
    assert!(pushed.contains(&"web_sources:s1".to_string()));
    assert!(pushed.contains(&"web_captures:c1".to_string()));
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name LIKE 'web_%'"
        ),
        0,
        "pushed rows leave the oplog"
    );
}

#[tokio::test]
async fn first_sight_of_the_capability_runs_one_catch_up_from_zero() {
    let conn = session_db();
    // The legacy cursor is already past rows the server held back.
    meta_set(&conn, "last_pull_seq", "50").unwrap();
    let dir = app_dir();

    let api = MockSyncApi::default();
    api.server_capabilities
        .lock()
        .unwrap()
        .push(WEB_CAPTURE_CAPABILITY.to_string());
    // 1st page: the ordinary incremental pull (nothing new, advertises the token).
    api.queue_pull_page(page(vec![], 50, &[WEB_CAPTURE_CAPABILITY]));
    // 2nd page: the since-zero catch-up. Corpus rows in it are ignored here.
    api.queue_pull_page(page(
        vec![
            capture_row("c1", "s1", 4),
            source_row("s1", 3),
            PullRow {
                table: "collections".to_string(),
                row_id: "ignored".to_string(),
                server_seq: 2,
                deleted: false,
                changed_at: 1,
                device_id: "remote".to_string(),
                payload: Some(json!({"id":"ignored","name":"X","created_at":1,"updated_at":1})),
            },
        ],
        4,
        &[WEB_CAPTURE_CAPABILITY],
    ));

    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");

    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM web_sources WHERE id='s1'"),
        1
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM web_captures WHERE id='c1'"),
        1
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM collections WHERE id='ignored'"),
        0,
        "the catch-up applies web rows only; corpus history stays with the shared cursor"
    );
    assert_eq!(
        meta_get_i64(&conn, "last_pull_seq").unwrap(),
        50,
        "the shared cursor is not moved by the catch-up"
    );
    assert!(!catchup_needed(&conn, EPOCH).unwrap(), "catch-up recorded");

    // Second cycle: the ordinary pull(s) repeat, the catch-up does not.
    let first_cycle_pulls = *api.plain_pull_calls.lock().unwrap();
    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    let second_cycle_pulls = *api.plain_pull_calls.lock().unwrap() - first_cycle_pulls;
    assert_eq!(
        second_cycle_pulls + 1,
        first_cycle_pulls,
        "exactly one catch-up pull in the first cycle, none in the second"
    );
}

#[tokio::test]
async fn no_catch_up_against_a_server_without_the_capability() {
    let conn = session_db();
    let dir = app_dir();
    let api = MockSyncApi::default();
    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    let first = *api.plain_pull_calls.lock().unwrap();
    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    assert_eq!(
        *api.plain_pull_calls.lock().unwrap(),
        first * 2,
        "a legacy server never triggers an extra pull"
    );
    assert!(catchup_needed(&conn, EPOCH).unwrap());
}

#[test]
fn a_capture_that_arrives_before_its_source_is_parked_then_applied() {
    let conn = session_db();
    let dir = app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    apply_page(&conn, &mut ctx, &[capture_row("c1", "s1", 4)], 4).expect("page 1");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM web_captures"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"), 1);

    apply_page(&conn, &mut ctx, &[source_row("s1", 5)], 5).expect("page 2");
    retry_pending_rows(&conn, &mut ctx, false).expect("retry");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM web_captures WHERE id='c1'"),
        1
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"), 0);
    // Applying remote rows never re-captures them for push.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_oplog"), 0);
}

#[test]
fn a_remote_source_tombstone_removes_its_captures_but_not_a_dirty_one() {
    let conn = session_db();
    let dir = app_dir();
    let mut ctx = ApplyContext::new(dir.path());
    apply_page(
        &conn,
        &mut ctx,
        &[source_row("s1", 3), capture_row("c1", "s1", 4)],
        4,
    )
    .expect("seed");

    apply_page(&conn, &mut ctx, &[tombstone("web_sources", "s1", 9)], 9).expect("tombstone");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM web_sources"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM web_captures"), 0);

    // A capture with a pending local edit defers the whole tombstone.
    apply_page(
        &conn,
        &mut ctx,
        &[source_row("s2", 10), capture_row("c2", "s2", 11)],
        11,
    )
    .expect("seed 2");
    conn.execute(
        "INSERT INTO sync_oplog(table_name, row_id, op, changed_at) VALUES ('web_captures','c2','U',1)",
        [],
    )
    .unwrap();
    apply_page(&conn, &mut ctx, &[tombstone("web_sources", "s2", 12)], 12).expect("tombstone 2");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM web_sources WHERE id='s2'"),
        1,
        "a dirty cascade child defers the tombstone"
    );
}
