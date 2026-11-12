//! Tests for the `entity-aliases-v1` row sync: capability record, push gating,
//! the one-time catch-up, and apply of the alias table. Mirrors
//! `web_capture_tests.rs`: the table travels in the ordinary stream, held back
//! from the push until the server advertises the token.

use serde_json::json;

use super::apply::{apply_page, retry_pending_rows, ApplyContext};
use super::engine::run_cycle;
use super::entity_aliases::{
    catchup_needed, clear_account_metadata, record_capability, supports_entity_aliases,
};
use super::http::{
    PullResponse, PullRow, ENTITY_ALIASES_V1_CAPABILITY as ENTITY_ALIASES_CAPABILITY,
};
use super::session::{meta_get_i64, meta_set};
use super::test_support::{new_synced_test_db, set_session_with_capture, MockSyncApi};
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

fn insert_alias(conn: &Connection, id: &str, key: &str) {
    conn.execute(
        "INSERT INTO entity_aliases(id, alias_key, alias, canonical, created_at, updated_at)
         VALUES (?1, ?2, 'Mdme. Cuevas', 'Carmela Cuevas', 1, 1)",
        [id, key],
    )
    .expect("insert alias");
}

fn alias_row(id: &str, key: &str, seq: i64) -> PullRow {
    PullRow {
        table: "entity_aliases".to_string(),
        row_id: id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: 2,
        device_id: "remote".to_string(),
        payload: Some(json!({
            "id": id, "alias_key": key, "alias": "Mdme. Cuevas",
            "canonical": "Carmela Cuevas", "created_at": 1, "updated_at": 2
        })),
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
    assert_eq!(ENTITY_ALIASES_CAPABILITY, "entity-aliases-v1");
}

#[test]
fn capability_is_trusted_only_for_the_epoch_that_advertised_it() {
    let conn = new_synced_test_db();
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    assert!(!supports_entity_aliases(&conn).unwrap(), "nothing recorded");

    record_capability(&conn, EPOCH, true).unwrap();
    assert!(supports_entity_aliases(&conn).unwrap());

    // A restored/rotated server (new epoch) must re-advertise.
    meta_set(&conn, "server_epoch", "other-epoch").unwrap();
    assert!(!supports_entity_aliases(&conn).unwrap());

    // A later ordinary response that omits the token (rolled-back server).
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    record_capability(&conn, EPOCH, false).unwrap();
    assert!(!supports_entity_aliases(&conn).unwrap());
}

#[test]
fn capability_recorded_only_from_an_ordinary_pull_and_scoped_to_the_epoch() {
    // The verdict is written by the ordinary pull loop (`pull::pull_loop`) and
    // read back only for the epoch that advertised it: a legacy epoch that
    // never sent the token must not inherit a newer epoch's verdict.
    let conn = session_db();
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    record_capability(&conn, EPOCH, true).unwrap();
    assert!(supports_entity_aliases(&conn).unwrap());
    meta_set(&conn, "server_epoch", "next-epoch").unwrap();
    assert!(
        !supports_entity_aliases(&conn).unwrap(),
        "the verdict never leaks across epochs"
    );
}

#[test]
fn clearing_account_metadata_forgets_capability_and_catchup() {
    let conn = new_synced_test_db();
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    record_capability(&conn, EPOCH, true).unwrap();
    super::entity_aliases::record_catchup_done(&conn, EPOCH).unwrap();
    assert!(!catchup_needed(&conn, EPOCH).unwrap());

    clear_account_metadata(&conn).unwrap();
    assert!(!supports_entity_aliases(&conn).unwrap());
    assert!(catchup_needed(&conn, EPOCH).unwrap());
}

#[tokio::test]
async fn alias_rows_stay_in_the_oplog_until_the_server_advertises_the_capability() {
    let conn = session_db();
    insert_alias(&conn, "alias:mdme. cuevas", "mdme. cuevas");
    let dir = app_dir();

    // A legacy server: no capability. The row must not be sent (an old server
    // would reject the whole batch) and must stay pending.
    let legacy = MockSyncApi::default();
    run_cycle(&legacy, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    assert_eq!(
        legacy.pushed_count(),
        0,
        "no alias row reaches a legacy server"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name = 'entity_aliases'"
        ),
        1,
        "alias rows are held, not dropped"
    );

    // The server is upgraded: the same cycle discovers the token on its pull
    // response and sends the held row.
    let modern = MockSyncApi::default();
    modern
        .server_capabilities
        .lock()
        .unwrap()
        .push(ENTITY_ALIASES_CAPABILITY.to_string());
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
    assert_eq!(
        pushed,
        vec!["entity_aliases:alias:mdme. cuevas".to_string()]
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name = 'entity_aliases'"
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
        .push(ENTITY_ALIASES_CAPABILITY.to_string());
    // 1st page: the ordinary incremental pull (nothing new, advertises the token).
    api.queue_pull_page(page(vec![], 50, &[ENTITY_ALIASES_CAPABILITY]));
    // 2nd page: the since-zero catch-up. Corpus rows in it are ignored here.
    api.queue_pull_page(page(
        vec![
            alias_row("alias:mdme. cuevas", "mdme. cuevas", 4),
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
        &[ENTITY_ALIASES_CAPABILITY],
    ));

    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM entity_aliases WHERE id = 'alias:mdme. cuevas'"
        ),
        1
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM collections WHERE id='ignored'"),
        0,
        "the catch-up applies alias rows only; corpus history stays with the shared cursor"
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
    assert!(
        second_cycle_pulls < first_cycle_pulls,
        "exactly one catch-up pull in the first cycle, none in the second \
         (first={first_cycle_pulls} pulls, second={second_cycle_pulls})"
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
fn pulled_alias_row_with_the_deterministic_id_converges() {
    // S6: both devices derive the same row id from the normalized key, so the
    // pulled row upserts onto the local one instead of parking on UNIQUE.
    let conn = session_db();
    let dir = app_dir();
    let mut ctx = ApplyContext::new(dir.path());
    insert_alias(&conn, "alias:mdme. cuevas", "mdme. cuevas");
    conn.execute_batch("DELETE FROM sync_oplog;")
        .expect("clear");

    apply_page(
        &conn,
        &mut ctx,
        &[alias_row("alias:mdme. cuevas", "mdme. cuevas", 7)],
        7,
    )
    .expect("apply");
    retry_pending_rows(&conn, &mut ctx, false).expect("retry");

    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM entity_aliases"),
        1,
        "the same alias converges to one row"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"),
        0,
        "nothing parks on the UNIQUE(alias_key)"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_oplog"),
        0,
        "applying remote rows never re-captures them for push"
    );
}
