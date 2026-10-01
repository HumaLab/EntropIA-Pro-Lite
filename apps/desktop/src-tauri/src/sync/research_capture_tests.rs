//! Tests for the inactive research capture layer: terminal-only seeding,
//! idempotence, generation-bound compare-and-delete acknowledgment, literal
//! prefix parsing, fail-closed malformed metadata and account cleanup. The
//! sync fixtures use the real `sync_*` schema; the research state fixtures are
//! temporary `estado.sqlite` files built with the real EntropIA-Agent schema.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;

use super::research_capture::{
    abort_delete_intent, acknowledge_outbox_entry, begin_delete_intent, clear_account_metadata,
    enqueue_terminal_job, outbox_entries, record_capability, seed_terminal_outbox,
    settle_delete_intent, supports_research, CAPABILITY_KEY, DELETE_INTENT_PREFIX, ENVELOPE_TABLE,
    OUTBOX_PREFIX, RECEIVE_PREFIX, RESEARCH_CAPABILITY,
};
use super::research_envelope::{RESEARCH_JOB_NOT_FOUND, RESEARCH_JOB_NOT_TERMINAL};

fn temp_workspace(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "entropia-research-capture-{}-{}-{}",
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

fn insert_job(conn: &Connection, id: &str, modo: &str, status: &str, close_reason: Option<&str>) {
    conn.execute(
        "INSERT INTO jobs (id, modo, pregunta, status, close_reason, config_snapshot,
                           corpus_snapshot_id, project, corpus, created_at, updated_at)
         VALUES (?1, ?2, '¿Pregunta?', ?3, ?4, '{}', 'snap-1', 'demo', 'desktop', 100, 200)",
        rusqlite::params![id, modo, status, close_reason],
    )
    .expect("insert job");
}

/// Terminal and non-terminal research jobs plus one foreign job kind.
fn mixed_state(path: &Path) -> Connection {
    let state = research_state(path);
    insert_job(&state, "j-done", "research", "done", Some("completed"));
    insert_job(&state, "j-failed", "research", "failed", Some("blocked"));
    insert_job(&state, "j-running", "research", "running", None);
    insert_job(&state, "j-gated", "research", "awaiting_human", None);
    insert_job(&state, "j-planned", "research", "planned", None);
    insert_job(&state, "j-paused", "research", "paused", None);
    insert_job(&state, "j-paper", "paper", "done", Some("completed"));
    state
}

fn sync_connection() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db");
    crate::sync::schema::ensure_sync_schema(&conn).expect("sync schema");
    set_meta(&conn, "capture_enabled", "1");
    conn
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .expect("set meta");
}

fn read_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM sync_meta WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

fn job_ids(conn: &Connection) -> Vec<String> {
    outbox_entries(conn)
        .expect("outbox entries")
        .into_iter()
        .map(|entry| entry.job_id)
        .collect()
}

#[test]
fn seeding_enqueues_only_terminal_research_jobs() {
    let root = temp_workspace("terminal-only");
    let sync = sync_connection();
    let state = mixed_state(&root.join("estado.sqlite"));

    let seeded = seed_terminal_outbox(&sync, &state).expect("seed");
    assert_eq!(seeded, 2);
    assert_eq!(
        job_ids(&sync),
        vec!["j-done".to_string(), "j-failed".to_string()]
    );

    // A direct enqueue of an active or missing job is rejected, never captured.
    let error = enqueue_terminal_job(&sync, &state, "j-running").expect_err("active job enqueued");
    assert_eq!(error.code, RESEARCH_JOB_NOT_TERMINAL);
    let error = enqueue_terminal_job(&sync, &state, "j-gated").expect_err("gated job enqueued");
    assert_eq!(error.code, RESEARCH_JOB_NOT_TERMINAL);
    let error = enqueue_terminal_job(&sync, &state, "j-missing").expect_err("missing job");
    assert_eq!(error.code, RESEARCH_JOB_NOT_FOUND);

    // A terminal job that closes later is captured on request.
    enqueue_terminal_job(&sync, &state, "j-done").expect("enqueue terminal");
    assert_eq!(job_ids(&sync).len(), 2);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn seeding_is_idempotent_and_skips_acknowledged_jobs() {
    let root = temp_workspace("idempotence");
    let sync = sync_connection();
    let state = mixed_state(&root.join("estado.sqlite"));

    assert_eq!(seed_terminal_outbox(&sync, &state).expect("first seed"), 2);
    let first_values: Vec<String> = outbox_entries(&sync)
        .expect("entries")
        .iter()
        .map(|entry| entry.acknowledgment.generation.clone())
        .collect();
    assert_eq!(seed_terminal_outbox(&sync, &state).expect("second seed"), 0);
    let second_values: Vec<String> = outbox_entries(&sync)
        .expect("entries")
        .iter()
        .map(|entry| entry.acknowledgment.generation.clone())
        .collect();
    assert_eq!(first_values, second_values, "pending work was replaced");

    // Acknowledged jobs (present in sync_row_versions) are never re-seeded.
    let entry = outbox_entries(&sync)
        .expect("entries")
        .into_iter()
        .find(|entry| entry.job_id == "j-done")
        .expect("j-done entry");
    assert!(acknowledge_outbox_entry(&sync, &entry.acknowledgment).expect("ack"));
    sync.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, 'j-done', 7)",
        rusqlite::params![ENVELOPE_TABLE],
    )
    .expect("record server row");
    assert_eq!(seed_terminal_outbox(&sync, &state).expect("third seed"), 0);
    assert_eq!(job_ids(&sync), vec!["j-failed".to_string()]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn acknowledgment_is_exact_compare_and_delete_on_the_generation() {
    let root = temp_workspace("generation-cas");
    let sync = sync_connection();
    let state = mixed_state(&root.join("estado.sqlite"));

    enqueue_terminal_job(&sync, &state, "j-done").expect("first enqueue");
    let first = outbox_entries(&sync)
        .expect("entries")
        .into_iter()
        .next()
        .expect("first entry");
    assert_eq!(first.job_id, "j-done");
    assert_eq!(first.generation, first.acknowledgment.generation);

    // A second capture replaces the generation; the stale acknowledgment is a
    // harmless no-op that can never remove the newer work.
    enqueue_terminal_job(&sync, &state, "j-done").expect("second enqueue");
    let second = outbox_entries(&sync)
        .expect("entries")
        .into_iter()
        .next()
        .expect("second entry");
    assert_ne!(first.generation, second.generation);
    assert!(!acknowledge_outbox_entry(&sync, &first.acknowledgment).expect("stale ack"));
    assert_eq!(job_ids(&sync), vec!["j-done".to_string()]);

    // The exact captured generation deletes once; replaying it changes nothing.
    assert!(acknowledge_outbox_entry(&sync, &second.acknowledgment).expect("fresh ack"));
    assert!(job_ids(&sync).is_empty());
    assert!(!acknowledge_outbox_entry(&sync, &second.acknowledgment).expect("replayed ack"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn outbox_entries_parse_by_literal_prefix_and_stable_order() {
    let sync = sync_connection();
    // A later job that changed earlier sorts first; prefix matching is literal.
    set_meta(&sync, &format!("{OUTBOX_PREFIX}j-b"), &stored_value(200));
    set_meta(&sync, &format!("{OUTBOX_PREFIX}j-a"), &stored_value(100));
    set_meta(&sync, "research_outbox", "decoy-no-colon");
    set_meta(&sync, "Research_outbox:j-x", "decoy-case");
    set_meta(&sync, "xresearch_outbox:j-y", "decoy-suffix");

    let entries = outbox_entries(&sync).expect("entries");
    let ids: Vec<&str> = entries.iter().map(|entry| entry.job_id.as_str()).collect();
    assert_eq!(ids, vec!["j-a", "j-b"]);
    assert_eq!(entries[0].changed_at, 100);
    assert_eq!(entries[1].changed_at, 200);
    assert!(entries.iter().all(|entry| entry.op == 'U'));
}

fn stored_value(changed_at: i64) -> String {
    serde_json::json!({
        "op": "U",
        "changed_at": changed_at,
        "generation": "3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9",
    })
    .to_string()
}

#[test]
fn malformed_outbox_and_capability_values_fail_closed() {
    let sync = sync_connection();
    set_meta(&sync, &format!("{OUTBOX_PREFIX}j-a"), "esto no es JSON");
    assert!(outbox_entries(&sync).is_err(), "garbage value must fail");

    set_meta(
        &sync,
        &format!("{OUTBOX_PREFIX}j-a"),
        r#"{"op":"U","changed_at":1}"#,
    );
    assert!(
        outbox_entries(&sync).is_err(),
        "a generation-less entry is malformed"
    );

    set_meta(
        &sync,
        &format!("{OUTBOX_PREFIX}j-a"),
        r#"{"op":"D","changed_at":1,"generation":"3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9"}"#,
    );
    let entries = outbox_entries(&sync).expect("delete tombstones parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].op, 'D',
        "the outbox parses the captured delete op"
    );

    set_meta(&sync, &format!("{OUTBOX_PREFIX}j-a"), &stored_value(-1));
    assert!(
        outbox_entries(&sync).is_err(),
        "a negative changed_at is malformed"
    );

    set_meta(&sync, OUTBOX_PREFIX, &stored_value(1));
    assert!(
        outbox_entries(&sync).is_err(),
        "a key without a job id is malformed"
    );

    set_meta(&sync, CAPABILITY_KEY, "tampoco es JSON");
    assert!(
        supports_research(&sync, "epoch-1").is_err(),
        "a malformed capability record fails closed"
    );
}

#[test]
fn capability_record_is_bound_to_the_server_epoch() {
    let sync = sync_connection();
    assert!(!supports_research(&sync, "epoch-1").expect("absent record"));

    record_capability(&sync, "epoch-1", true).expect("record");
    assert_eq!(RESEARCH_CAPABILITY, "research-envelope-v1");
    assert!(supports_research(&sync, "epoch-1").expect("same epoch"));
    assert!(
        !supports_research(&sync, "epoch-2").expect("stale epoch"),
        "discovery from another epoch never enables the transport"
    );

    record_capability(&sync, "epoch-2", false).expect("re-record");
    assert!(!supports_research(&sync, "epoch-2").expect("not advertised"));
    assert!(!supports_research(&sync, "epoch-1").expect("old epoch gone"));
}

#[test]
fn account_cleanup_removes_research_metadata_and_nothing_else() {
    let sync = sync_connection();
    set_meta(&sync, &format!("{OUTBOX_PREFIX}j-a"), &stored_value(1));
    set_meta(&sync, &format!("{OUTBOX_PREFIX}j-b"), &stored_value(2));
    set_meta(&sync, CAPABILITY_KEY, "{}");
    set_meta(&sync, &format!("{RECEIVE_PREFIX}j-a"), "queued");
    set_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-a"), "intent");
    set_meta(&sync, "writing_outbox:doc-1", &stored_value(3));
    set_meta(&sync, "writing_capability", "{}");
    set_meta(&sync, "capture_enabled", "1");
    set_meta(&sync, "last_pull_seq", "9");
    // Literal prefix matching: neither the bare key nor look-alikes die.
    set_meta(&sync, "research_outbox", "decoy-no-colon");
    set_meta(&sync, "Research_outbox:j-x", "decoy-case");

    clear_account_metadata(&sync).expect("clear");

    assert_eq!(read_meta(&sync, &format!("{OUTBOX_PREFIX}j-a")), None);
    assert_eq!(read_meta(&sync, &format!("{OUTBOX_PREFIX}j-b")), None);
    assert_eq!(read_meta(&sync, CAPABILITY_KEY), None);
    assert_eq!(read_meta(&sync, &format!("{RECEIVE_PREFIX}j-a")), None);
    assert_eq!(
        read_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-a")),
        None
    );
    assert!(read_meta(&sync, "writing_outbox:doc-1").is_some());
    assert!(read_meta(&sync, "writing_capability").is_some());
    assert_eq!(read_meta(&sync, "capture_enabled").as_deref(), Some("1"));
    assert_eq!(read_meta(&sync, "last_pull_seq").as_deref(), Some("9"));
    assert_eq!(
        read_meta(&sync, "research_outbox").as_deref(),
        Some("decoy-no-colon")
    );
    assert_eq!(
        read_meta(&sync, "Research_outbox:j-x").as_deref(),
        Some("decoy-case")
    );
}

#[test]
fn capture_is_a_no_op_without_sync_schema_or_when_disabled() {
    let root = temp_workspace("no-op");
    let state = mixed_state(&root.join("estado.sqlite"));

    // No sync schema at all: nothing fails and nothing is captured.
    let bare = Connection::open_in_memory().expect("in-memory db");
    enqueue_terminal_job(&bare, &state, "j-done").expect("no-op enqueue");
    assert_eq!(seed_terminal_outbox(&bare, &state).expect("no-op seed"), 0);

    // Capture disabled (or an apply in flight): same no-op.
    let disabled = sync_connection();
    set_meta(&disabled, "capture_enabled", "0");
    enqueue_terminal_job(&disabled, &state, "j-done").expect("disabled enqueue");
    assert_eq!(
        seed_terminal_outbox(&disabled, &state).expect("disabled seed"),
        0
    );
    assert!(job_ids(&disabled).is_empty());

    let applying = sync_connection();
    set_meta(&applying, "applying", "1");
    enqueue_terminal_job(&applying, &state, "j-done").expect("applying enqueue");
    assert_eq!(
        seed_terminal_outbox(&applying, &state).expect("applying seed"),
        0
    );
    assert!(job_ids(&applying).is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_intent_capture_parses_and_aborts_or_settles_an_idempotent_tombstone() {
    let root = temp_workspace("delete-intent");
    let sync = sync_connection();
    let state = mixed_state(&root.join("estado.sqlite"));

    // The intent is captured only for an existing research job.
    begin_delete_intent(&sync, &state, "j-paper").expect("foreign job");
    begin_delete_intent(&sync, &state, "j-missing").expect("missing job");
    assert_eq!(
        read_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-paper")),
        None
    );
    assert!(outbox_entries(&sync).expect("outbox").is_empty());

    // A begun delete parses as an op 'D' intent and an explicit abort removes
    // it without enqueueing anything.
    begin_delete_intent(&sync, &state, "j-gated").expect("begin");
    let raw = read_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-gated")).expect("intent record");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("intent json");
    assert_eq!(parsed["op"].as_str(), Some("D"));
    assert_eq!(parsed["job_id"].as_str(), Some("j-gated"));
    abort_delete_intent(&sync, "j-gated").expect("abort");
    assert_eq!(
        read_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-gated")),
        None
    );
    assert!(
        outbox_entries(&sync).expect("outbox").is_empty(),
        "a failed delete request never enqueues a delete"
    );

    // A delete that did not happen settles to nothing but the intent removal.
    begin_delete_intent(&sync, &state, "j-running").expect("begin");
    settle_delete_intent(&sync, &state, "j-running").expect("failed delete");
    assert_eq!(
        read_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-running")),
        None
    );
    assert!(outbox_entries(&sync).expect("outbox").is_empty());

    // A successful engine delete settles into ONE idempotent tombstone whose
    // generation is acknowledged exactly like any other capture.
    begin_delete_intent(&sync, &state, "j-done").expect("begin");
    state
        .execute("DELETE FROM jobs WHERE id = 'j-done'", [])
        .expect("engine delete");
    settle_delete_intent(&sync, &state, "j-done").expect("settle");
    settle_delete_intent(&sync, &state, "j-done").expect("idempotent settle");
    assert_eq!(
        read_meta(&sync, &format!("{DELETE_INTENT_PREFIX}j-done")),
        None
    );
    let entries = outbox_entries(&sync).expect("outbox");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].job_id, "j-done");
    assert_eq!(entries[0].op, 'D');
    assert!(acknowledge_outbox_entry(&sync, &entries[0].acknowledgment).expect("ack"));
    assert!(outbox_entries(&sync).expect("outbox").is_empty());

    // Without a begun intent nothing settles; a corrupt intent fails closed.
    settle_delete_intent(&sync, &state, "j-failed").expect("no intent");
    assert!(outbox_entries(&sync).expect("outbox").is_empty());
    set_meta(
        &sync,
        &format!("{DELETE_INTENT_PREFIX}j-failed"),
        "{no es json",
    );
    let error = settle_delete_intent(&sync, &state, "j-failed").expect_err("corrupt intent");
    assert_eq!(error.code, "research_delete_intent_corrupt");

    // Capture off: no intent is ever recorded.
    let disabled = sync_connection();
    set_meta(&disabled, "capture_enabled", "0");
    begin_delete_intent(&disabled, &state, "j-failed").expect("no-op");
    settle_delete_intent(&disabled, &state, "j-failed").expect("no-op");
    assert_eq!(
        read_meta(&disabled, &format!("{DELETE_INTENT_PREFIX}j-failed")),
        None
    );
    assert!(outbox_entries(&disabled).expect("outbox").is_empty());

    let _ = std::fs::remove_dir_all(&root);
}
