//! Behavior tests for the inactive writing pull page staging (W-NET5).
//!
//! Every test runs against MockSyncApi and a throwaway schema; the mock's
//! defaults stay neutral (no capability advertisement), so each test opts in
//! explicitly to exactly the state it exercises.

use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Value};

use super::http::{PullResponse, PullRow, WRITING_ENVELOPE_V1_CAPABILITY};
use super::session::{
    meta_delete, meta_get, meta_set, write_sync_session, SYNC_SESSION_INCARNATION_KEY,
};
use super::test_support::{new_synced_test_db, MockSyncApi};
use super::writing_pull::{prepare_writing_pull, pull_writing_page, stage_writing_pull_page};
use super::writing_receive::{
    enqueue_writing_receive, prepare_writing_receive, queued_writing_receives,
    settle_writing_receive,
};
use crate::writing::sync_capture::{
    outbox_entries, record_capability, ENVELOPE_TABLE, PULL_CURSOR_KEY,
};
use crate::writing::sync_transport::catchup_needed;

fn session(conn: &Connection) {
    write_sync_session(
        conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("session");
    meta_set(conn, "server_epoch", "epoch-a").expect("epoch");
}

fn discovered(conn: &Connection) {
    session(conn);
    record_capability(conn, "epoch-a", true).expect("capability");
}

fn writing_row(seq: i64, document_id: &str) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: document_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(json!({ "id": document_id, "text": seq })),
    }
}

fn writing_tombstone(seq: i64, document_id: &str) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: document_id.to_string(),
        server_seq: seq,
        deleted: true,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: None,
    }
}

fn corpus_row(seq: i64, table: &str, row_id: &str) -> PullRow {
    PullRow {
        table: table.to_string(),
        row_id: row_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 100,
        device_id: "device-other".to_string(),
        payload: Some(json!({ "id": row_id })),
    }
}

fn page(rows: Vec<PullRow>, next_since: i64, has_more: bool) -> PullResponse {
    PullResponse {
        rows,
        next_since,
        has_more,
        schema_tag: "0023_sync_ids".to_string(),
        server_epoch: "epoch-a".to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: vec![WRITING_ENVELOPE_V1_CAPABILITY.to_string()],
    }
}

fn cursor(conn: &Connection) -> Option<i64> {
    let raw = meta_get(conn, PULL_CURSOR_KEY).expect("read staging cursor");
    raw.map(|raw| {
        serde_json::from_str::<Value>(&raw)
            .expect("cursor json")
            .get("since")
            .and_then(Value::as_i64)
            .expect("cursor since")
    })
}

#[tokio::test]
async fn missing_capability_never_reaches_the_wire() {
    let conn = new_synced_test_db();
    session(&conn); // no capability advertised for epoch-a
    let api = MockSyncApi::default();
    api.queue_pull_page(page(vec![writing_row(5, "doc-1")], 5, true));

    let error = pull_writing_page(&conn, &api, "token", 10)
        .await
        .expect_err("capability gate");

    assert_eq!(error.code, "writing_pull_capability_missing");
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 0);
    assert_eq!(
        api.pull_pages.lock().unwrap().len(),
        1,
        "no pull was served"
    );
    assert!(queued_writing_receives(&conn).expect("queue").is_empty());
    assert_eq!(cursor(&conn), None);
}

#[tokio::test]
async fn missing_session_incarnation_never_reaches_the_wire() {
    let conn = new_synced_test_db();
    discovered(&conn);
    meta_delete(&conn, SYNC_SESSION_INCARNATION_KEY).expect("drop incarnation");
    let api = MockSyncApi::default();
    api.queue_pull_page(page(vec![writing_row(5, "doc-1")], 5, true));

    let error = pull_writing_page(&conn, &api, "token", 10)
        .await
        .expect_err("incarnation gate");

    assert_eq!(error.code, "writing_pull_missing_session");
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 0);
    assert_eq!(
        api.pull_pages.lock().unwrap().len(),
        1,
        "no pull was served"
    );
    assert!(queued_writing_receives(&conn).expect("queue").is_empty());
}

#[tokio::test]
async fn opted_in_pull_is_exact_and_stages_the_page() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let mut api = MockSyncApi::default();
    api.server_epoch = "epoch-a".to_string();
    *api.server_capabilities.lock().unwrap() = vec![WRITING_ENVELOPE_V1_CAPABILITY.to_string()];
    api.queue_pull_page(page(vec![writing_row(5, "doc-1")], 5, true));

    let staged = pull_writing_page(&conn, &api, "token", 10)
        .await
        .expect("opted-in pull");

    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 1);
    assert_eq!(staged.staged_document_ids, vec!["doc-1".to_string()]);
    assert_eq!(queued_writing_receives(&conn).expect("queue").len(), 1);
    assert!(api.pushed.lock().unwrap().is_empty(), "no push happened");
    assert!(
        outbox_entries(&conn).expect("outbox").is_empty(),
        "no seed_outbox"
    );

    // The next page request continues from the exact writing staging cursor:
    // the mock's terminal page echoes the `since` it received (5).
    let staged = pull_writing_page(&conn, &api, "token", 10)
        .await
        .expect("second pull");
    assert_eq!(*api.writing_capability_pull_calls.lock().unwrap(), 2);
    assert_eq!(staged.next_since, 5);
    assert!(staged.staged_document_ids.is_empty());
    assert!(!staged.catchup_recorded, "queue still holds doc-1");
}

#[test]
fn corpus_rows_are_returned_while_writing_rows_are_queued() {
    let conn = new_synced_test_db();
    discovered(&conn);
    meta_set(&conn, "last_pull_seq", "7").expect("shared cursor");
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![
            corpus_row(10, "items", "item-1"),
            writing_row(15, "doc-1"),
            corpus_row(20, "assets", "asset-1"),
        ],
        20,
        true,
    );

    let staged = stage_writing_pull_page(&conn, &prepared, &response).expect("stage");

    let corpus_ids: Vec<&str> = staged
        .corpus_rows
        .iter()
        .map(|row| row.row_id.as_str())
        .collect();
    assert_eq!(
        corpus_ids,
        vec!["item-1", "asset-1"],
        "corpus rows preserved"
    );
    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].document_id, "doc-1");
    assert_eq!(staged.staged_document_ids, vec!["doc-1".to_string()]);
    assert_eq!(
        meta_get(&conn, "last_pull_seq")
            .expect("shared cursor")
            .as_deref(),
        Some("7"),
        "shared last_pull_seq is never advanced by writing staging"
    );
    assert!(!staged.catchup_recorded);
}

#[test]
fn cursor_advances_with_durable_enqueue_of_the_page() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![writing_row(5, "doc-a"), writing_row(6, "doc-b")],
        30,
        true,
    );

    let staged = stage_writing_pull_page(&conn, &prepared, &response).expect("stage");

    assert_eq!(staged.next_since, 30);
    assert_eq!(cursor(&conn), Some(30));
    assert_eq!(queued_writing_receives(&conn).expect("queue").len(), 2);
}

#[test]
fn failed_enqueue_rolls_back_cursor_and_queue_changes() {
    let conn = new_synced_test_db();
    discovered(&conn);
    conn.execute_batch(
        "CREATE TEMP TRIGGER fail_stage_row
           BEFORE INSERT ON sync_meta
           WHEN NEW.key = 'writing_receive:doc-b'
           BEGIN SELECT RAISE(ABORT, 'injected stage failure'); END;",
    )
    .expect("trigger");
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![writing_row(5, "doc-a"), writing_row(6, "doc-b")],
        6,
        true,
    );

    let error = stage_writing_pull_page(&conn, &prepared, &response).expect_err("rollback");

    assert_eq!(error.code, "writing_pull_local_state");
    assert!(
        queued_writing_receives(&conn).expect("queue").is_empty(),
        "doc-a rolled back with the failed page"
    );
    assert_eq!(
        cursor(&conn),
        None,
        "cursor rolled back with the failed page"
    );
    assert!(catchup_needed(&conn, "epoch-a").expect("catchup"));
}

#[test]
fn failed_cursor_write_rolls_back_the_page_queue() {
    let conn = new_synced_test_db();
    discovered(&conn);
    conn.execute_batch(
        "CREATE TEMP TRIGGER fail_cursor
           BEFORE INSERT ON sync_meta
           WHEN NEW.key = 'writing_pull_cursor'
           BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;",
    )
    .expect("trigger");
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let response = page(vec![writing_row(5, "doc-a")], 5, true);

    let error = stage_writing_pull_page(&conn, &prepared, &response).expect_err("rollback");

    assert_eq!(error.code, "writing_pull_local_state");
    assert!(
        queued_writing_receives(&conn).expect("queue").is_empty(),
        "durable enqueue rolled back with the failed cursor write"
    );
    assert_eq!(cursor(&conn), None);
}

#[test]
fn more_pages_never_complete_catchup_while_the_queue_is_nonempty() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let staged = stage_writing_pull_page(
        &conn,
        &prepared,
        &page(vec![writing_row(5, "doc-1")], 5, false),
    )
    .expect("stage");
    assert!(!staged.catchup_recorded, "queued rows block completion");
    assert!(catchup_needed(&conn, "epoch-a").expect("catchup"));

    // A terminal empty page still cannot complete catch-up: doc-1 is an
    // unresolved queued row (deferred rows are queued rows too).
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let staged = stage_writing_pull_page(&conn, &prepared, &page(vec![], 5, false)).expect("stage");
    assert!(!staged.catchup_recorded);
    assert!(catchup_needed(&conn, "epoch-a").expect("catchup"));
}

#[test]
fn empty_resolved_page_records_catchup() {
    let conn = new_synced_test_db();
    discovered(&conn);
    conn.execute_batch(
        "INSERT INTO _migrations(name, applied_at) VALUES('0035_writing_workspace', 1);
         INSERT INTO _migrations(name, applied_at) VALUES('0036_writing_journal', 2);",
    )
    .expect("writing migration head");

    // Stage a tombstone page, then resolve it through the explicit settle call.
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let staged = stage_writing_pull_page(
        &conn,
        &prepared,
        &page(vec![writing_tombstone(5, "doc-1")], 5, true),
    )
    .expect("stage");
    assert!(!staged.catchup_recorded);
    let queued = prepare_writing_receive(&conn, "doc-1")
        .expect("prepare receive")
        .expect("queued row");
    settle_writing_receive(&conn, &queued, Path::new("unused-data-root")).expect("settle");
    assert!(queued_writing_receives(&conn).expect("queue").is_empty());

    // The terminal page is empty and the queue is resolved: catch-up completes.
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let staged = stage_writing_pull_page(&conn, &prepared, &page(vec![], 5, false)).expect("stage");
    assert!(staged.catchup_recorded);
    assert!(!catchup_needed(&conn, "epoch-a").expect("catchup"));
    assert_eq!(cursor(&conn), Some(5));
}

#[test]
fn replaying_the_same_page_is_idempotent() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    let response = page(
        vec![writing_row(5, "doc-1"), corpus_row(6, "items", "item-1")],
        6,
        true,
    );

    let first = stage_writing_pull_page(&conn, &prepared, &response).expect("first stage");
    let first_queue = queued_writing_receives(&conn).expect("queue");
    assert_eq!(first_queue.len(), 1);

    let second = stage_writing_pull_page(&conn, &prepared, &response).expect("replay");
    let second_queue = queued_writing_receives(&conn).expect("queue");

    assert_eq!(second.staged_document_ids, first.staged_document_ids);
    assert_eq!(second.next_since, first.next_since);
    assert_eq!(second_queue.len(), 1);
    assert_eq!(
        second_queue[0].captured_value,
        first_queue[0].captured_value
    );
    assert_eq!(cursor(&conn), Some(6));
}

#[test]
fn session_change_rejects_stale_staging() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("new session");

    let error = stage_writing_pull_page(
        &conn,
        &prepared,
        &page(vec![writing_row(5, "doc-1")], 5, true),
    )
    .expect_err("stale staging");

    assert_eq!(error.code, "writing_pull_stale_staging");
    assert!(
        queued_writing_receives(&conn).expect("queue").is_empty(),
        "old rows are never staged or applied"
    );
    assert_eq!(cursor(&conn), None);
}

#[test]
fn stale_staging_cursor_from_a_previous_session_is_rejected() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    stage_writing_pull_page(
        &conn,
        &prepared,
        &page(vec![writing_row(5, "doc-1")], 5, true),
    )
    .expect("stage");
    assert_eq!(cursor(&conn), Some(5));

    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("new session");

    let error = prepare_writing_pull(&conn, 10).expect_err("stale cursor");
    assert_eq!(error.code, "writing_pull_stale_staging");
}

#[test]
fn staged_rows_round_trip_through_the_receive_queue() {
    let conn = new_synced_test_db();
    discovered(&conn);
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");
    stage_writing_pull_page(
        &conn,
        &prepared,
        &page(vec![writing_tombstone(7, "doc-1")], 7, true),
    )
    .expect("stage");

    let queued = queued_writing_receives(&conn).expect("queue");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].document_id, "doc-1");
    assert_eq!(queued[0].server_seq, 7);
    assert_eq!(queued[0].device_id, "device-other");
    assert!(
        prepare_writing_receive(&conn, "doc-1")
            .expect("prepare receive")
            .is_some(),
        "the real queue reader accepts the staged value shape"
    );
}

#[test]
fn staging_replays_rows_enqueued_by_the_receive_queue() {
    let conn = new_synced_test_db();
    discovered(&conn);
    enqueue_writing_receive(&conn, &writing_row(5, "doc-1")).expect("enqueue");
    let before = queued_writing_receives(&conn).expect("queue");
    let prepared = prepare_writing_pull(&conn, 10).expect("prepare");

    let staged = stage_writing_pull_page(
        &conn,
        &prepared,
        &page(vec![writing_row(5, "doc-1")], 5, true),
    )
    .expect("stage");

    let after = queued_writing_receives(&conn).expect("queue");
    assert_eq!(staged.staged_document_ids, vec!["doc-1".to_string()]);
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].captured_value, before[0].captured_value,
        "both writers agree on the durable queue identity"
    );
    assert_eq!(cursor(&conn), Some(5));
}
