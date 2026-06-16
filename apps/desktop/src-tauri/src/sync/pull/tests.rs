//! Pull-cycle tests (DESIGN §4.3, §4.5, §4.9): seeding idempotence, seed-after
//! -pull ordering, the paginated loop, and epoch reconciliation against the
//! in-memory mock.

use super::*;
use crate::sync::capture::ensure_capture;
use crate::sync::http::{PullResponse, PullRow};
use crate::sync::test_support::{new_synced_test_db, MockSyncApi};

use rusqlite::Connection;
use serde_json::json;

fn capturing_db() -> Connection {
    let conn = new_synced_test_db();
    ensure_capture(&conn).expect("ensure capture");
    crate::sync::test_support::set_session_with_capture(&conn);
    conn
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn tmp_app_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("assets")).expect("assets dir");
    dir
}

fn upsert_row(table: &str, row_id: &str, server_seq: i64, payload: serde_json::Value) -> PullRow {
    PullRow {
        table: table.to_string(),
        row_id: row_id.to_string(),
        server_seq,
        deleted: false,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: Some(payload),
    }
}

fn page(rows: Vec<PullRow>, next_since: i64, has_more: bool, epoch: &str) -> PullResponse {
    PullResponse {
        rows,
        next_since,
        has_more,
        schema_tag: "0023_sync_ids".to_string(),
        server_epoch: epoch.to_string(),
        server_now_ms: 1_700_000_000_000,
    }
}

// --------------------------------------------------------------------------
// Seeding (DESIGN §4.5)
// --------------------------------------------------------------------------

#[test]
fn seeding_enqueues_inserts_for_unversioned_rows_and_is_idempotent() {
    let conn = capturing_db();
    // Pre-existing rows that the server has not versioned.
    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    seed_account(&conn, "acc-1").expect("seed");
    let oplog_after_first = count(&conn, "SELECT COUNT(*) FROM sync_oplog WHERE op='I'");
    assert!(oplog_after_first >= 2, "collection + item seeded");

    // Re-run (simulated crash before marking seeded was cleared): idempotent.
    conn.execute("DELETE FROM sync_meta WHERE key='seeded_account'", [])
        .unwrap();
    seed_account(&conn, "acc-1").expect("re-seed");
    // The coalescer collapses duplicate (table,row_id) 'I' entries, so the row
    // set is unchanged: still exactly one seed entry per row.
    let distinct_rows: i64 = count(
        &conn,
        "SELECT COUNT(*) FROM (SELECT DISTINCT table_name, row_id FROM sync_oplog WHERE op='I')",
    );
    assert_eq!(distinct_rows, 2, "no new distinct rows after re-seed");
}

#[test]
fn seeding_skips_rows_the_server_already_knows() {
    let conn = capturing_db();
    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    // The server already knows item i1 (a prior pull populated row_versions).
    conn.execute(
        "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','i1',5)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    seed_account(&conn, "acc-1").expect("seed");
    // collections.c1 seeded (unknown), items.i1 NOT seeded (known to server).
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name='collections'"
        ),
        1
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name='items'"
        ),
        0,
        "row known to the server is not re-seeded"
    );
}

#[test]
fn seeding_is_guarded_by_seeded_account() {
    let conn = capturing_db();
    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
    seed_account(&conn, "acc-1").expect("seed");
    let after_first = count(&conn, "SELECT COUNT(*) FROM sync_oplog");
    // Second call with the SAME account is a no-op (guard short-circuits).
    seed_account(&conn, "acc-1").expect("noop");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_oplog"), after_first);
}

// --------------------------------------------------------------------------
// Epoch reconciliation (DESIGN §4.9)
// --------------------------------------------------------------------------

#[test]
fn check_epoch_persists_first_then_resets_on_mismatch() {
    let conn = capturing_db();
    // Seed a cursor + versions to prove the reset wipes them.
    crate::sync::session::meta_set_i64(&conn, "last_pull_seq", 42).unwrap();
    conn.execute(
        "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','i1',5)",
        [],
    )
    .unwrap();

    // First epoch: just persisted, no reset.
    assert!(!check_epoch(&conn, "epoch-A").unwrap());
    assert_eq!(
        crate::sync::session::meta_get(&conn, "server_epoch")
            .unwrap()
            .as_deref(),
        Some("epoch-A")
    );

    // Same epoch: no reset.
    assert!(!check_epoch(&conn, "epoch-A").unwrap());
    assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 42);

    // Different epoch: reset (cursor=0, versions cleared, new epoch persisted).
    assert!(check_epoch(&conn, "epoch-B").unwrap());
    assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_row_versions"), 0);
    assert_eq!(
        crate::sync::session::meta_get(&conn, "server_epoch")
            .unwrap()
            .as_deref(),
        Some("epoch-B")
    );
}

// --------------------------------------------------------------------------
// Pull loop against the mock
// --------------------------------------------------------------------------

#[tokio::test]
async fn pull_loop_applies_paginated_pages() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let api = MockSyncApi::default();

    // Page 1 (has_more) → collection; page 2 (terminal) → item under it.
    api.queue_pull_page(page(
        vec![upsert_row(
            "collections",
            "c1",
            1,
            json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
        )],
        1,
        true,
        "mock-epoch",
    ));
    api.queue_pull_page(page(
        vec![upsert_row(
            "items",
            "i1",
            2,
            json!({"id":"i1","title":"A","collection_id":"c1","created_at":1,"updated_at":1}),
        )],
        2,
        false,
        "mock-epoch",
    ));

    let outcome = pull_loop(&api, "tok", "0023_sync_ids", &conn, dir.path())
        .await
        .expect("pull loop");
    assert_eq!(outcome.pages, 2);
    assert_eq!(outcome.applied, 2);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM collections WHERE id='c1'"),
        1
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 1);
    // Cursor at the last page's next_since.
    assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 2);
}

#[tokio::test]
async fn pull_loop_parks_child_then_drains_when_parent_lands_next_page() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let api = MockSyncApi::default();

    // Page 1: child item arrives BEFORE its collection → parked.
    api.queue_pull_page(page(
        vec![upsert_row(
            "items",
            "i1",
            1,
            json!({"id":"i1","title":"Orphan","collection_id":"c1","created_at":1,"updated_at":1}),
        )],
        1,
        true,
        "mock-epoch",
    ));
    // Page 2: the parent collection (re-sequenced after its child).
    api.queue_pull_page(page(
        vec![upsert_row(
            "collections",
            "c1",
            2,
            json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
        )],
        2,
        false,
        "mock-epoch",
    ));

    let outcome = pull_loop(&api, "tok", "0023_sync_ids", &conn, dir.path())
        .await
        .expect("pull loop");
    assert!(outcome.parked >= 1, "child parked on page 1");
    // After the parent lands, the post-page retry drains the parked child.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"), 0);
}

#[tokio::test]
async fn pull_loop_reconciles_on_409_cursor_ahead() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let api = MockSyncApi::default();

    // Stale cursor + versions; the first pull 409s → reconcile resets them.
    crate::sync::session::meta_set_i64(&conn, "last_pull_seq", 999).unwrap();
    conn.execute(
        "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','old',5)",
        [],
    )
    .unwrap();
    api.set_cursor_ahead(1);
    // After reconciliation the loop restarts at since=0 and gets a terminal page.
    api.queue_pull_page(page(
        vec![upsert_row(
            "collections",
            "c1",
            1,
            json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
        )],
        1,
        false,
        "mock-epoch",
    ));

    let outcome = pull_loop(&api, "tok", "0023_sync_ids", &conn, dir.path())
        .await
        .expect("pull loop");
    assert!(outcome.reconciled, "409 triggered reconciliation");
    // Old version wiped, cursor reset then advanced to the real page.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_row_versions WHERE row_id='old'"
        ),
        0
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM collections WHERE id='c1'"),
        1
    );
}

#[tokio::test]
async fn pull_loop_reconciles_on_epoch_mismatch_end_to_end() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let api = MockSyncApi::default();

    // The client already knows epoch-A with a stale cursor.
    crate::sync::session::meta_set(&conn, "server_epoch", "epoch-A").unwrap();
    crate::sync::session::meta_set_i64(&conn, "last_pull_seq", 50).unwrap();
    conn.execute(
        "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','gone',9)",
        [],
    )
    .unwrap();

    // First page comes back with a DIFFERENT epoch (a restore). check_epoch
    // resets and the loop restarts; the second page (epoch-B) then applies.
    api.queue_pull_page(page(Vec::new(), 0, false, "epoch-B"));
    api.queue_pull_page(page(
        vec![upsert_row(
            "collections",
            "c1",
            1,
            json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
        )],
        1,
        false,
        "epoch-B",
    ));

    let outcome = pull_loop(&api, "tok", "0023_sync_ids", &conn, dir.path())
        .await
        .expect("pull loop");
    assert!(outcome.reconciled, "epoch mismatch reconciled");
    assert_eq!(
        crate::sync::session::meta_get(&conn, "server_epoch")
            .unwrap()
            .as_deref(),
        Some("epoch-B")
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_row_versions WHERE row_id='gone'"
        ),
        0
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM collections WHERE id='c1'"),
        1
    );
}

#[tokio::test]
async fn pull_loop_cuts_on_remote_schema_tag_ahead() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let api = MockSyncApi::default();

    // The server reports a schema_tag AHEAD of the local head → cut the loop.
    api.queue_pull_page(PullResponse {
        rows: Vec::new(),
        next_since: 1,
        has_more: true,
        schema_tag: "0099_future".to_string(),
        server_epoch: "mock-epoch".to_string(),
        server_now_ms: 1,
    });

    let outcome = pull_loop(&api, "tok", "0023_sync_ids", &conn, dir.path())
        .await
        .expect("pull loop");
    assert!(outcome.schema_cut, "remote schema_tag ahead cuts the loop");
}

#[tokio::test]
async fn pull_loop_skips_rows_already_at_version() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let api = MockSyncApi::default();
    conn.execute(
        "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('collections','c1',5)",
        [],
    )
    .unwrap();
    api.queue_pull_page(page(
        vec![upsert_row(
            "collections",
            "c1",
            5,
            json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
        )],
        5,
        false,
        "mock-epoch",
    ));

    let outcome = pull_loop(&api, "tok", "0023_sync_ids", &conn, dir.path())
        .await
        .expect("pull loop");
    assert_eq!(outcome.skipped, 1, "row already at its version is skipped");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM collections"), 0);
}
