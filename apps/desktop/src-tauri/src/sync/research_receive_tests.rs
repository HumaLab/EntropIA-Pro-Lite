//! Tests for the durable research receive queue: crash-safe staging in
//! `sync_meta`, session-scope binding, exact captured-value compare-and-delete,
//! bounded retained errors for unsupported rows, dirty deferral retention, and
//! end-to-end settlement into the separate research state database. Fixtures
//! are temporary real EntropIA-Agent `estado.sqlite` files plus the real sync
//! schema fixture.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;
use serde_json::{json, Value};
use uuid::Uuid;

use super::research_capture::{has_pending_outbox_entry, ENVELOPE_TABLE, OUTBOX_PREFIX};
use super::research_receive::{
    enqueue_research_receive, prepare_research_receive, queued_research_receives,
    settle_research_receive,
};
use super::research_transport::PullApplyOutcome;
use super::session::{clear_sync_state, write_sync_session, SYNC_SESSION_INCARNATION_KEY};
use crate::sync::http::PullRow;
use crate::sync::schema::ensure_sync_schema;
use crate::sync::session::{meta_get, meta_set};

fn temp_workspace(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "entropia-research-receive-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        name
    ));
    std::fs::create_dir_all(&dir).expect("temp workspace");
    dir
}

fn research_state(path: &Path) -> Connection {
    entropia_agent::estado::EstadoDb::abrir(path.to_str().expect("utf-8 path"))
        .expect("estado schema");
    Connection::open(path).expect("raw state connection")
}

/// Real sync schema fixture with a live session (own device `device-own`).
fn sync_connection() -> Connection {
    let conn = crate::sync::test_support::new_synced_test_db();
    session(&conn);
    conn
}

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

fn envelope_payload(job_id: &str) -> Value {
    json!({
        "id": job_id,
        "envelope_version": 1,
        "job": {
            "id": job_id,
            "status": "done",
            "close_reason": "completed",
            "title": "Estudio de campo",
            "question": "¿Pregunta de investigación?",
            "project": "demo",
            "corpus_snapshot_id": null,
            "created_at": 100,
            "updated_at": 200
        },
        "report": {"version": 1, "content_json": {"markdown": "# Informe"}},
        "sources": [
            {"source_id": "c1", "evidence_id": "c1", "item_id": "i1",
             "chunk_id": "c1", "text_hash": null, "title": "Documento A"}
        ],
        "report_file": null
    })
}

fn upsert(job_id: &str, seq: i64, payload: Option<Value>) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload,
    }
}

fn tombstone(job_id: &str, seq: i64) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq: seq,
        deleted: true,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: None,
    }
}

fn queued_ids(conn: &Connection) -> Vec<String> {
    queued_research_receives(conn)
        .expect("queue read")
        .into_iter()
        .map(|row| row.job_id)
        .collect()
}

#[test]
fn queued_receive_is_durable_and_survives_reopening_the_database() {
    let root = temp_workspace("durable");
    let path = root.join("queue.sqlite");
    {
        let conn = Connection::open(&path).expect("create");
        ensure_sync_schema(&conn).expect("schema");
        session(&conn);
        enqueue_research_receive(&conn, &upsert("job-1", 7, Some(envelope_payload("job-1"))))
            .expect("enqueue");
    }

    // A crash before the research database is applied leaves the staged row.
    let reopened = Connection::open(&path).expect("reopen");
    let queued = queued_research_receives(&reopened).expect("read");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].job_id, "job-1");
    assert_eq!(queued[0].server_seq, 7);
    assert_eq!(queued[0].attempts, 0);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn queue_scope_is_bound_to_the_current_session() {
    let conn = sync_connection();
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        [SYNC_SESSION_INCARNATION_KEY],
    )
    .expect("remove incarnation");
    let error = enqueue_research_receive(&conn, &upsert("job-1", 1, None)).expect_err("no session");
    assert_eq!(error.code, "research_receive_missing_session");
    session(&conn);

    enqueue_research_receive(&conn, &upsert("job-1", 3, Some(envelope_payload("job-1"))))
        .expect("enqueue");
    let raw = meta_get(&conn, "research_receive:job-1")
        .expect("raw")
        .expect("stored");
    let parsed: Value = serde_json::from_str(&raw).expect("json");
    assert_eq!(parsed["scope"]["account_id"], json!("account-a"));
    assert_eq!(
        parsed["scope"]["server_url"],
        json!("https://sync.example.test")
    );
    assert_eq!(parsed["scope"]["device_id"], json!("device-own"));
    assert_eq!(parsed["scope"]["server_epoch"], json!("epoch-a"));
    assert!(
        Uuid::parse_str(
            parsed["scope"]["session_incarnation"]
                .as_str()
                .expect("uuid")
        )
        .is_ok(),
        "the scope carries the session incarnation"
    );

    // A prepared row from a past session can never be settled, and a re-login
    // invalidates the preparation.
    let prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("re-login");
    meta_set(&conn, "server_epoch", "epoch-a").expect("epoch");
    let error = settle_research_receive(
        &conn,
        &Connection::open_in_memory().expect("state"),
        &temp_workspace("scope"),
        &prepared,
    )
    .expect_err("stale scope");
    assert_eq!(error.code, "research_receive_scope_changed");
    let error = prepare_research_receive(&conn, "job-1").expect_err("stale prepare");
    assert_eq!(error.code, "research_receive_scope_changed");
    assert_eq!(
        queued_ids(&conn),
        vec!["job-1".to_string()],
        "the row is retained for the next matching session"
    );
}

#[test]
fn queue_replaces_by_sequence_and_settles_with_exact_captured_value_cas() {
    let conn = sync_connection();
    enqueue_research_receive(&conn, &upsert("job-1", 3, Some(envelope_payload("job-1"))))
        .expect("first");
    enqueue_research_receive(&conn, &upsert("job-1", 3, Some(envelope_payload("job-1"))))
        .expect("equal replay");
    let mut conflicting = upsert("job-1", 3, None);
    conflicting.payload = Some(json!({"id": "job-1", "otro": true}));
    let error = enqueue_research_receive(&conn, &conflicting).expect_err("conflict");
    assert_eq!(error.code, "research_receive_payload_conflict");
    enqueue_research_receive(&conn, &upsert("job-1", 8, Some(envelope_payload("job-1"))))
        .expect("newer");
    enqueue_research_receive(&conn, &upsert("job-1", 4, Some(envelope_payload("job-1"))))
        .expect("older ignored");

    let queued = queued_research_receives(&conn).expect("read");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].server_seq, 8);

    // Settlement compares the exact captured value: a newer staged row can
    // never be removed by a stale preparation.
    let stale_prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    enqueue_research_receive(&conn, &upsert("job-1", 9, Some(envelope_payload("job-1"))))
        .expect("newer mid-flight");
    let error = settle_research_receive(
        &conn,
        &Connection::open_in_memory().expect("state"),
        &temp_workspace("cas"),
        &stale_prepared,
    )
    .expect_err("stale captured value");
    assert_eq!(error.code, "research_receive_queue_changed");
    assert_eq!(
        queued_research_receives(&conn).expect("read")[0].server_seq,
        9
    );

    // The fresh preparation settles and removes exactly its own stage.
    let prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    let root = temp_workspace("cas-apply");
    let state = research_state(&root.join("estado.sqlite"));
    let outcome =
        settle_research_receive(&conn, &state, &root.join("artifacts"), &prepared).expect("settle");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_id: None
        }
    );
    assert!(queued_research_receives(&conn).expect("read").is_empty());
    assert!(prepare_research_receive(&conn, "job-1")
        .expect("gone")
        .is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unsupported_rows_are_retained_with_a_bounded_error_and_no_version_ack() {
    let conn = sync_connection();
    // A missing upsert payload and a malformed envelope both reach the queue
    // (they are wire facts) and are retained as unsupported, never dropped.
    enqueue_research_receive(&conn, &upsert("job-1", 5, None)).expect("missing payload");
    let huge_field = "x".repeat(900);
    let mut malformed = serde_json::Map::new();
    malformed.insert("id".to_string(), json!("job-2"));
    malformed.insert(huge_field, json!(true));
    enqueue_research_receive(&conn, &upsert("job-2", 5, Some(Value::Object(malformed))))
        .expect("malformed envelope");

    let root = temp_workspace("unsupported");
    let state = research_state(&root.join("estado.sqlite"));
    let artifacts = root.join("artifacts");

    let prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    let outcome = settle_research_receive(&conn, &state, &artifacts, &prepared).expect("settle");
    assert_eq!(
        outcome,
        PullApplyOutcome::Unsupported {
            reason: "research upsert row without payload".to_string()
        }
    );

    let prepared = prepare_research_receive(&conn, "job-2")
        .expect("prepare")
        .expect("queued");
    let outcome = settle_research_receive(&conn, &state, &artifacts, &prepared).expect("settle");
    assert!(
        matches!(outcome, PullApplyOutcome::Unsupported { .. }),
        "a malformed envelope is unsupported: {outcome:?}"
    );

    // Both rows are retained with bounded errors and attempt counts, and no
    // row version was acknowledged.
    let queued = queued_research_receives(&conn).expect("read");
    assert_eq!(queued.len(), 2);
    for row in &queued {
        assert_eq!(row.attempts, 1);
        assert!(row.last_error.as_ref().expect("error").len() <= 512);
        assert!(!row.last_error.as_ref().expect("error").is_empty());
    }
    assert_eq!(
        queued[1].last_error.as_ref().expect("error").len(),
        512,
        "an oversized error is truncated, never grown"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sync_row_versions WHERE table_name = ?1",
            [ENVELOPE_TABLE],
            |row| row.get::<_, i64>(0)
        )
        .expect("versions"),
        0,
        "an unsupported row is never acknowledged"
    );

    // Retention is cumulative: the next attempt keeps counting.
    let prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    let _ = settle_research_receive(&conn, &state, &artifacts, &prepared).expect("second settle");
    assert_eq!(
        queued_research_receives(&conn)
            .expect("read")
            .iter()
            .find(|row| row.job_id == "job-1")
            .expect("job-1")
            .attempts,
        2
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dirty_deferral_retains_the_queue_entry() {
    let conn = sync_connection();
    let root = temp_workspace("dirty");
    let state = research_state(&root.join("estado.sqlite"));
    let artifacts = root.join("artifacts");
    meta_set(
        &conn,
        &format!("{OUTBOX_PREFIX}job-1"),
        &json!({"op":"U","changed_at":1,"generation":"3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9"})
            .to_string(),
    )
    .expect("outbox");

    enqueue_research_receive(&conn, &tombstone("job-1", 6)).expect("enqueue");
    let prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    let outcome = settle_research_receive(&conn, &state, &artifacts, &prepared).expect("settle");
    assert!(matches!(outcome, PullApplyOutcome::Deferred { .. }));

    // The deferred row is retained with its bounded error, and the pending
    // local work is untouched.
    let queued = queued_research_receives(&conn).expect("read");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].attempts, 1);
    assert!(queued[0].last_error.is_some());
    assert!(
        has_pending_outbox_entry(&conn, "job-1").expect("outbox"),
        "a received row never clears local outbox work"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sync_row_versions WHERE table_name = ?1",
            [ENVELOPE_TABLE],
            |row| row.get::<_, i64>(0)
        )
        .expect("versions"),
        0
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn settle_applies_the_row_to_the_research_database_and_removes_the_stage() {
    let conn = sync_connection();
    let root = temp_workspace("end-to-end");
    let state = research_state(&root.join("estado.sqlite"));
    let artifacts = root.join("artifacts");

    enqueue_research_receive(&conn, &upsert("job-1", 5, Some(envelope_payload("job-1"))))
        .expect("enqueue");
    let prepared = prepare_research_receive(&conn, "job-1")
        .expect("prepare")
        .expect("queued");
    let outcome = settle_research_receive(&conn, &state, &artifacts, &prepared).expect("settle");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_id: None
        }
    );

    // The readable terminal projection landed in the separate state database.
    let (status, pregunta): (String, String) = state
        .query_row(
            "SELECT status, pregunta FROM jobs WHERE id = 'job-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("job row");
    assert_eq!(status, "done");
    assert_eq!(pregunta, "¿Pregunta de investigación?");
    let artifacts_count: i64 = state
        .query_row(
            "SELECT COUNT(*) FROM artifacts WHERE job_id = 'job-1'",
            [],
            |row| row.get(0),
        )
        .expect("artifacts");
    assert_eq!(artifacts_count, 3);
    assert!(queued_research_receives(&conn).expect("read").is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn logout_clears_research_receive_and_delete_intent_keys_only() {
    let conn = sync_connection();
    meta_set(&conn, "research_receive:job-1", "queued").expect("queue");
    meta_set(&conn, "research_delete_intent:job-1", "intent").expect("intent");
    meta_set(&conn, &format!("{OUTBOX_PREFIX}job-1"), "outbox").expect("outbox");
    meta_set(&conn, "research_capability", "{}").expect("capability");
    // Literal prefix matching: hyphen look-alikes and bare keys survive.
    meta_set(&conn, "research-receive:job-1", "decoy-hyphen").expect("decoy");
    meta_set(&conn, "researchXreceive:job-1", "decoy-x").expect("decoy");
    meta_set(&conn, "research_receive", "decoy-bare").expect("decoy");

    clear_sync_state(&conn).expect("logout");
    assert_eq!(
        meta_get(&conn, "research_receive:job-1").expect("read"),
        None
    );
    assert_eq!(
        meta_get(&conn, "research_delete_intent:job-1").expect("read"),
        None
    );
    assert_eq!(
        meta_get(&conn, &format!("{OUTBOX_PREFIX}job-1")).expect("read"),
        None
    );
    assert_eq!(meta_get(&conn, "research_capability").expect("read"), None);
    assert_eq!(
        meta_get(&conn, "research-receive:job-1")
            .expect("read")
            .as_deref(),
        Some("decoy-hyphen")
    );
    assert_eq!(
        meta_get(&conn, "researchXreceive:job-1")
            .expect("read")
            .as_deref(),
        Some("decoy-x")
    );
    assert_eq!(
        meta_get(&conn, "research_receive")
            .expect("read")
            .as_deref(),
        Some("decoy-bare")
    );
}
