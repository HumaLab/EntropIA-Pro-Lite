//! Tests for the offline research receive semantics: create/update/idempotent
//! projection apply, stale and own-device echo behavior, malformed retention
//! without partial writes, dirty-local deferral, the divergent-upsert conflict
//! journal, and remote tombstone delete idempotence. Fixtures are temporary
//! `estado.sqlite` files built with the real EntropIA-Agent schema plus the
//! real sync schema fixture.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;
use serde_json::{json, Value};

use super::research_blobs::{
    ResearchBlobPending, ResearchBlobPendingKind, ResearchReportUploadPlan,
};
use super::research_capture::{
    has_pending_outbox_entry, outbox_entries, ENVELOPE_TABLE, OUTBOX_PREFIX,
};
use super::research_envelope::{snapshot_job_conn, ResearchEnvelopeV1, REPORT_FILE_REL_PATH};
use super::research_transport::{
    apply_lww_lost_winner, apply_pulled_row, build_push_changes,
    build_push_changes_with_report_prover, settle_applied_push_draft, LwwLostSettlement,
    PullApplyOutcome, PulledResearchRow, PushDraftSettlement, RESEARCH_CONFLICT_DIVERGENT_UPSERT,
    RESEARCH_CONFLICT_REMOTE_DELETE,
};
use crate::sync::http::{PullRow, PushResult};
use crate::sync::session::write_sync_session;

fn temp_workspace(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "entropia-research-transport-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        name
    ));
    std::fs::create_dir_all(&dir).expect("temp workspace");
    dir
}

/// Builds the real `estado.sqlite` schema (EntropIA-Agent owns the DDL).
fn research_state(path: &Path) -> Connection {
    entropia_agent::estado::EstadoDb::abrir(path.to_str().expect("utf-8 path"))
        .expect("estado schema");
    Connection::open(path).expect("raw state connection")
}

/// Real sync schema fixture with a live session (own device `device-own`).
fn sync_connection() -> Connection {
    let conn = crate::sync::test_support::new_synced_test_db();
    write_sync_session(
        &conn,
        "https://sync.example.test",
        "account-a",
        "reader@example.test",
        "device-own",
    )
    .expect("session");
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

fn insert_job_row(
    conn: &Connection,
    id: &str,
    modo: &str,
    status: &str,
    close_reason: Option<&str>,
    pregunta: &str,
    created_at: i64,
    updated_at: i64,
) {
    conn.execute(
        "INSERT INTO jobs (id, modo, pregunta, plan_json, status, close_reason, config_snapshot,
                           corpus_snapshot_id, project, corpus, created_at, updated_at)
         VALUES (?1, ?2, ?3, '{}', ?4, ?5, '{}', 'snap-1', 'demo', 'desktop', ?6, ?7)",
        rusqlite::params![
            id,
            modo,
            pregunta,
            status,
            close_reason,
            created_at,
            updated_at
        ],
    )
    .expect("insert job");
}

fn insert_artifact_row(
    conn: &Connection,
    job_id: &str,
    id: &str,
    kind: &str,
    version: i64,
    content: &str,
) {
    conn.execute(
        "INSERT INTO artifacts (id, job_id, tipo, path, version, created_at, content_json)
         VALUES (?1, ?2, ?3, '', ?4, 150, ?5)",
        rusqlite::params![id, job_id, kind, version, content],
    )
    .expect("insert artifact");
}

/// A terminal research job as the engine leaves it at close: the readable
/// artifacts plus billable calls, checkpoints, gates and events that the
/// receive side must never recreate.
fn seed_full_job(state: &Connection, job_id: &str) {
    insert_job_row(
        state,
        job_id,
        "research",
        "done",
        Some("completed"),
        "¿Pregunta de investigación?",
        100,
        200,
    );
    insert_artifact_row(
        state,
        job_id,
        &format!("req-{job_id}"),
        "request",
        1,
        r#"{"title":"Estudio de campo","question":"¿Pregunta de investigación?","project":"demo"}"#,
    );
    insert_artifact_row(
        state,
        job_id,
        &format!("rep-{job_id}"),
        "report",
        1,
        r##"{"report":{"title":"vigente"},"markdown":"# Vigente"}"##,
    );
    let evidence = json!([
        {"id":"c1","item_id":"i1","title":"Documento A"},
        {"id":"c1@8000","chunk_id":"c1","item_id":"i1","title":"Documento A"},
        {"id":"c2","item_id":"i2","title":"Documento B","text_hash":"abc123"},
        {"id":"zotero:ABCD","title":"Paper Z","provenance":"zotero"}
    ]);
    insert_artifact_row(
        state,
        job_id,
        &format!("arc-{job_id}"),
        "archive",
        1,
        &json!({"summary":"síntesis","evidence":evidence}).to_string(),
    );
    // Engine-owned history the projection must not recreate.
    for statement in [
        format!(
            "INSERT INTO llm_calls (id, job_id, rol, modelo, costo, created_at)
             VALUES ('call-{job_id}', '{job_id}', 'report', 'modelo', 0.5, 150)"
        ),
        format!(
            "INSERT INTO stages (id, job_id, tipo, titulo, status, checkpoint)
             VALUES ('stage-{job_id}', '{job_id}', 'report', 'Informe', 'completed', 'checkpoint')"
        ),
        format!(
            "INSERT INTO job_events (id, job_id, tipo, payload, timestamp)
             VALUES ('event-{job_id}', '{job_id}', 'created', '{{}}', 150)"
        ),
        format!(
            "INSERT INTO queries (id, job_id, consulta, created_at)
             VALUES ('query-{job_id}', '{job_id}', 'consulta', 150)"
        ),
        format!(
            "INSERT INTO human_decisions (id, job_id, stage_id, alcance, decision, timestamp)
             VALUES ('gate-{job_id}', '{job_id}', 'stage-{job_id}', 'report', 'pending', 150)"
        ),
    ] {
        state.execute_batch(&statement).expect("engine-owned rows");
    }
}

/// The engine's report update: the previous version turns obsolete.
fn bump_report(state: &Connection, job_id: &str) {
    state
        .execute(
            "UPDATE artifacts SET obsolete = 1 WHERE job_id = ?1 AND tipo = 'report'",
            [job_id],
        )
        .expect("obsolete previous report");
    insert_artifact_row(
        state,
        job_id,
        &format!("rep2-{job_id}"),
        "report",
        2,
        r##"{"report":{"title":"vigente 2"},"markdown":"# Vigente 2"}"##,
    );
    state
        .execute("UPDATE jobs SET updated_at = 300 WHERE id = ?1", [job_id])
        .expect("touch job");
}

fn envelope_of(state: &Connection, artifacts_root: &Path, job_id: &str) -> ResearchEnvelopeV1 {
    snapshot_job_conn(state, artifacts_root, job_id).expect("snapshot")
}

fn upsert_row(job_id: &str, seq: i64, envelope: &ResearchEnvelopeV1) -> PulledResearchRow {
    PulledResearchRow {
        job_id: job_id.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(
            serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value"),
        ),
    }
}

fn own_upsert_row(job_id: &str, seq: i64, envelope: &ResearchEnvelopeV1) -> PulledResearchRow {
    let mut row = upsert_row(job_id, seq, envelope);
    row.device_id = "device-own".to_string();
    row
}

fn tombstone_row(job_id: &str, seq: i64) -> PulledResearchRow {
    PulledResearchRow {
        job_id: job_id.to_string(),
        server_seq: seq,
        deleted: true,
        changed_at: seq * 1_000,
        device_id: "device-other".to_string(),
        payload: None,
    }
}

fn count(state: &Connection, table: &str, job_id: &str) -> i64 {
    state
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE job_id = ?1"),
            [job_id],
            |row| row.get(0),
        )
        .expect("count")
}

fn jobs_count(state: &Connection) -> i64 {
    state
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("count jobs")
}

fn artifact_kinds(state: &Connection, job_id: &str) -> Vec<(String, i64, String)> {
    let mut stmt = state
        .prepare(
            "SELECT tipo, version, content_json FROM artifacts WHERE job_id = ?1 ORDER BY tipo",
        )
        .expect("prepare");
    let rows = stmt
        .query_map([job_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query artifacts")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("artifact rows");
    rows
}

fn conflicts(conn: &Connection) -> Vec<(String, String, String, String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT id, table_name, row_id, reason, loser_payload FROM sync_conflicts
              ORDER BY id",
        )
        .expect("prepare");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .expect("query conflicts")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("conflict rows");
    rows
}

fn recorded_server_seq(conn: &Connection, job_id: &str) -> Option<i64> {
    conn.query_row(
        "SELECT server_seq FROM sync_row_versions WHERE table_name = ?1 AND row_id = ?2",
        rusqlite::params![ENVELOPE_TABLE, job_id],
        |row| row.get(0),
    )
    .ok()
}

#[test]
fn create_update_and_idempotent_receive_replace_the_terminal_projection() {
    let root = temp_workspace("create-update");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");

    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 5, &envelope),
    )
    .expect("create");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_id: None
        }
    );

    // The readable projection is exactly the job summary plus the
    // deterministic request/report/archive artifacts.
    let kinds = artifact_kinds(&target, "job-1");
    assert_eq!(
        kinds
            .iter()
            .map(|(kind, ..)| kind.as_str())
            .collect::<Vec<_>>(),
        vec!["archive", "report", "request"]
    );
    assert_eq!(kinds[1].1, 1, "the report artifact keeps its version");
    let plan: Value = serde_json::from_str(
        &target
            .query_row("SELECT plan_json FROM jobs WHERE id = 'job-1'", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("plan"),
    )
    .expect("plan json");
    assert_eq!(plan["step"], json!(7), "the plan reads as closed");
    // Nothing billable, gated or transient is recreated.
    for table in [
        "llm_calls",
        "stages",
        "job_events",
        "human_decisions",
        "queries",
    ] {
        assert_eq!(
            count(&target, table, "job-1"),
            0,
            "{table} must not be recreated"
        );
    }
    // The full aggregate round-trips: report content and source manifest.
    assert_eq!(
        envelope_of(&target, &target_artifacts, "job-1"),
        envelope,
        "the applied projection re-snapshots to the same envelope"
    );
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(5));

    // Update: a newer report version replaces the projection wholesale.
    bump_report(&source, "job-1");
    let updated = envelope_of(&source, &source_artifacts, "job-1");
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 6, &updated),
    )
    .expect("update");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: false,
            conflict_id: None
        }
    );
    let kinds = artifact_kinds(&target, "job-1");
    assert_eq!(kinds.len(), 3, "the old report version row is replaced");
    assert_eq!(kinds[1].1, 2);
    assert_eq!(envelope_of(&target, &target_artifacts, "job-1"), updated);

    // Idempotent receive: the same aggregate again changes nothing.
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 7, &updated),
    )
    .expect("replay");
    assert_eq!(outcome, PullApplyOutcome::NoOp);
    assert_eq!(envelope_of(&target, &target_artifacts, "job-1"), updated);
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(7));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stale_rows_are_skipped_and_own_device_rows_only_echo_with_a_local_job() {
    let root = temp_workspace("stale-echo");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    seed_full_job(&source, "job-2");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");
    let envelope_two = envelope_of(&source, &source_artifacts, "job-2");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");
    apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 5, &envelope),
    )
    .expect("seed target");

    // Stale sequences never touch state.
    for seq in [5, 4, 1] {
        let outcome = apply_pulled_row(
            &sync,
            &target,
            &target_artifacts,
            "device-own",
            &upsert_row("job-1", seq, &envelope),
        )
        .expect("stale");
        assert_eq!(outcome, PullApplyOutcome::Stale);
    }
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(5));

    // An own-device row is an echo only while the local job exists, and an
    // echo never clears the pending outbox generation.
    set_meta(
        &sync,
        &format!("{OUTBOX_PREFIX}job-1"),
        &json!({"op":"U","changed_at":1,"generation":"3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9"})
            .to_string(),
    );
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &own_upsert_row("job-1", 6, &envelope),
    )
    .expect("echo");
    assert_eq!(outcome, PullApplyOutcome::OwnChangeObserved);
    assert_eq!(envelope_of(&target, &target_artifacts, "job-1"), envelope);
    assert!(
        has_pending_outbox_entry(&sync, "job-1").expect("outbox"),
        "only exact generation settlement may clear the outbox"
    );
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(6));

    // A missing local job is never an echo: it is restored.
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &own_upsert_row("job-2", 6, &envelope_two),
    )
    .expect("restore");
    assert_eq!(
        outcome,
        PullApplyOutcome::Applied {
            created: true,
            conflict_id: None
        }
    );
    assert_eq!(
        envelope_of(&target, &target_artifacts, "job-2"),
        envelope_two
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn malformed_and_unsupported_rows_never_write_partial_state() {
    let root = temp_workspace("malformed");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");
    let valid: Value =
        serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");

    let mut unsupported_version = valid.clone();
    unsupported_version["envelope_version"] = json!(2);
    let mut mismatched = valid.clone();
    mismatched["id"] = json!("job-9");
    mismatched["job"]["id"] = json!("job-9");
    let mut internal_mismatch = valid.clone();
    internal_mismatch["id"] = json!("job-otro");
    let mut bad_report = valid.clone();
    bad_report["report"]["content_json"] = json!(5);
    let mut bad_sources = valid.clone();
    bad_sources["sources"] = json!([
        {"source_id": "c1", "evidence_id": "c1"},
        {"source_id": "otro", "evidence_id": "c1"}
    ]);

    let cases: Vec<(&str, Option<Value>)> = vec![
        ("research upsert row without payload", None),
        (
            "failed to deserialize research envelope",
            Some(json!({"id": "job-1"})),
        ),
        (
            "unsupported research envelope version",
            Some(unsupported_version),
        ),
        ("does not match row", Some(mismatched)),
        ("must equal job.id", Some(internal_mismatch)),
        (
            "report.content_json must be a JSON object",
            Some(bad_report),
        ),
        ("duplicate source evidence id", Some(bad_sources)),
    ];
    for (expected, payload) in cases {
        let row = PulledResearchRow {
            job_id: "job-1".to_string(),
            server_seq: 9,
            deleted: false,
            changed_at: 9_000,
            device_id: "device-other".to_string(),
            payload,
        };
        let outcome = apply_pulled_row(&sync, &target, &target_artifacts, "device-own", &row)
            .expect("classify");
        match outcome {
            PullApplyOutcome::Unsupported { reason } => {
                assert!(
                    reason.contains(expected),
                    "reason {reason:?} lacks {expected:?}"
                )
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
        // Never a partial write: no projection rows and no version ack.
        assert_eq!(jobs_count(&target), 0);
        assert_eq!(recorded_server_seq(&sync, "job-1"), None);
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dirty_local_research_defers_the_incoming_row() {
    let root = temp_workspace("dirty-defer");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");
    apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 5, &envelope),
    )
    .expect("seed target");
    set_meta(
        &sync,
        &format!("{OUTBOX_PREFIX}job-1"),
        &json!({"op":"U","changed_at":1,"generation":"3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9"})
            .to_string(),
    );

    // A dirty tombstone defers and preserves the local job and its file.
    let job_dir = target_artifacts.join("job-1");
    std::fs::create_dir_all(&job_dir).expect("job dir");
    std::fs::write(job_dir.join("report.md"), b"# local").expect("report file");
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &tombstone_row("job-1", 6),
    )
    .expect("dirty tombstone");
    assert_eq!(
        outcome,
        PullApplyOutcome::Deferred {
            reason: "pending local research outbox work defers the remote delete".to_string()
        }
    );
    assert!(
        job_dir.join("report.md").exists(),
        "the local report survives"
    );
    assert_eq!(jobs_count(&target), 1);
    assert!(
        conflicts(&sync).is_empty(),
        "a deferred row journals nothing"
    );

    // A dirty upsert whose guarded local snapshot is invalid defers too.
    target
        .execute(
            "UPDATE artifacts SET content_json = 'no-es-json'
              WHERE job_id = 'job-1' AND tipo = 'report'",
            [],
        )
        .expect("corrupt local report");
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 7, &envelope),
    )
    .expect("dirty upsert");
    match outcome {
        PullApplyOutcome::Deferred { reason } => {
            assert!(reason.contains("local snapshot is not valid"), "{reason}")
        }
        other => panic!("expected Deferred, got {other:?}"),
    }
    assert_eq!(jobs_count(&target), 1);
    assert!(conflicts(&sync).is_empty());
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(5));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn divergent_upsert_journals_the_loser_and_applies_the_winner() {
    let root = temp_workspace("divergent");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let local = envelope_of(&source, &source_artifacts, "job-1");
    bump_report(&source, "job-1");
    let winner = envelope_of(&source, &source_artifacts, "job-1");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");
    apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 5, &local),
    )
    .expect("seed target");
    set_meta(
        &sync,
        &format!("{OUTBOX_PREFIX}job-1"),
        &json!({"op":"U","changed_at":1,"generation":"3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9"})
            .to_string(),
    );

    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 6, &winner),
    )
    .expect("divergent");
    let conflict_id = match outcome {
        PullApplyOutcome::Applied {
            created: false,
            conflict_id: Some(conflict_id),
        } => conflict_id,
        other => panic!("expected Applied with a conflict, got {other:?}"),
    };

    // Deterministic conflict id: reason + row id + loser fingerprint.
    assert_eq!(
        conflict_id,
        format!(
            "{RESEARCH_CONFLICT_DIVERGENT_UPSERT}-job-1-{}",
            local.fingerprint_sha256().expect("fingerprint")
        )
    );
    let rows = conflicts(&sync);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, conflict_id);
    assert_eq!(rows[0].1, ENVELOPE_TABLE);
    assert_eq!(rows[0].2, "job-1");
    assert_eq!(rows[0].3, RESEARCH_CONFLICT_DIVERGENT_UPSERT);
    assert_eq!(
        rows[0].4,
        local.to_canonical_json().expect("canonical loser"),
        "the generic conflict UI can surface the opaque loser payload"
    );
    // The server winner landed and the pending generation is untouched.
    assert_eq!(envelope_of(&target, &target_artifacts, "job-1"), winner);
    assert!(
        has_pending_outbox_entry(&sync, "job-1").expect("outbox"),
        "a pulled row never clears the outbox"
    );
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(6));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn remote_tombstone_journals_the_loser_and_deletes_idempotently() {
    let root = temp_workspace("tombstone");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");
    seed_full_job(&source, "job-2");
    let envelope_two = envelope_of(&source, &source_artifacts, "job-2");
    apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 5, &envelope),
    )
    .expect("seed job-1");
    apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-2", 5, &envelope_two),
    )
    .expect("seed job-2");
    // The local job owns a rendered report file on disk.
    let job_dir = target_artifacts.join("job-1");
    std::fs::create_dir_all(&job_dir).expect("job dir");
    std::fs::write(job_dir.join("report.md"), b"# local").expect("report file");
    let local = envelope_of(&target, &target_artifacts, "job-1");

    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &tombstone_row("job-1", 6),
    )
    .expect("tombstone");
    let conflict_id = match outcome {
        PullApplyOutcome::Applied {
            created: false,
            conflict_id: Some(conflict_id),
        } => conflict_id,
        other => panic!("expected Applied with a conflict, got {other:?}"),
    };
    assert_eq!(
        conflict_id,
        format!(
            "{RESEARCH_CONFLICT_REMOTE_DELETE}-job-1-{}",
            local.fingerprint_sha256().expect("fingerprint")
        )
    );
    let rows = conflicts(&sync);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].3, RESEARCH_CONFLICT_REMOTE_DELETE);
    assert_eq!(
        rows[0].4,
        local.to_canonical_json().expect("canonical loser"),
        "the losing envelope survives the deletion"
    );

    // Only the job-owned projection and its report file are deleted.
    assert_eq!(jobs_count(&target), 1, "only job-1 is deleted");
    for table in [
        "artifacts",
        "llm_calls",
        "stages",
        "job_events",
        "queries",
        "human_decisions",
    ] {
        assert_eq!(count(&target, table, "job-1"), 0, "{table} job-1 rows");
    }
    assert_eq!(count(&target, "artifacts", "job-2"), 3);
    assert!(
        !job_dir.join("report.md").exists(),
        "the report file is deleted"
    );

    // Idempotent replay: a second tombstone writes nothing new.
    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &tombstone_row("job-1", 7),
    )
    .expect("replay tombstone");
    assert_eq!(outcome, PullApplyOutcome::NoOp);
    assert_eq!(conflicts(&sync).len(), 1);
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(7));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn active_and_foreign_jobs_are_never_replaced_or_deleted() {
    let root = temp_workspace("guards");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");
    insert_job_row(
        &target,
        "job-foreign",
        "paper",
        "done",
        Some("completed"),
        "otro motor",
        100,
        200,
    );
    insert_job_row(
        &target,
        "job-active",
        "research",
        "running",
        None,
        "en ejecución",
        100,
        200,
    );

    let foreign_payload: Value = {
        let mut payload: Value =
            serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value");
        payload["id"] = json!("job-foreign");
        payload["job"]["id"] = json!("job-foreign");
        payload
    };
    let mut foreign = upsert_row("job-foreign", 5, &envelope);
    foreign.payload = Some(foreign_payload);
    let outcome = apply_pulled_row(&sync, &target, &target_artifacts, "device-own", &foreign)
        .expect("foreign upsert");
    assert!(matches!(outcome, PullApplyOutcome::Unsupported { .. }));
    assert_eq!(count(&target, "artifacts", "job-foreign"), 0);

    let outcome = apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &tombstone_row("job-active", 5),
    )
    .expect("active tombstone");
    assert!(matches!(outcome, PullApplyOutcome::Deferred { .. }));
    let mut active = upsert_row("job-active", 5, &envelope);
    let active_payload: Value = {
        let mut payload: Value =
            serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value");
        payload["id"] = json!("job-active");
        payload["job"]["id"] = json!("job-active");
        payload
    };
    active.payload = Some(active_payload);
    let outcome = apply_pulled_row(&sync, &target, &target_artifacts, "device-own", &active)
        .expect("active upsert");
    assert!(matches!(outcome, PullApplyOutcome::Deferred { .. }));
    assert_eq!(jobs_count(&target), 2, "both guarded jobs survive");

    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────────── push side ───────────────────────────────

const GENERATION_ONE: &str = "3f0c2f84-7b27-4f64-8f2e-6f3a61a2b1c9";
const GENERATION_TWO: &str = "9c1d3a52-4e8b-4a2c-9d7f-1b6e5c0a8f44";

/// One coalesced outbox entry exactly as capture stores it.
fn enqueue_outbox(conn: &Connection, job_id: &str, op: &str, changed_at: i64, generation: &str) {
    set_meta(
        conn,
        &format!("{OUTBOX_PREFIX}{job_id}"),
        &json!({"op": op, "changed_at": changed_at, "generation": generation}).to_string(),
    );
}

fn write_report(root: &Path, job_id: &str, bytes: &[u8]) {
    let job_dir = root.join(job_id);
    std::fs::create_dir_all(&job_dir).expect("job dir");
    std::fs::write(job_dir.join(REPORT_FILE_REL_PATH), bytes).expect("report file");
}

fn applied_result(job_id: &str, server_seq: i64) -> PushResult {
    PushResult {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        status: "applied".to_string(),
        server_seq,
        winner: None,
    }
}

fn winner_pull_row(job_id: &str, server_seq: i64, envelope: &ResearchEnvelopeV1) -> PullRow {
    PullRow {
        table: ENVELOPE_TABLE.to_string(),
        row_id: job_id.to_string(),
        server_seq,
        deleted: false,
        changed_at: server_seq * 1_000,
        device_id: "device-other".to_string(),
        payload: Some(
            serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value"),
        ),
    }
}

#[test]
fn u_drafts_carry_the_canonical_payload_and_the_proven_upload_plan() {
    let root = temp_workspace("draft-u");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    write_report(&source_artifacts, "job-1", b"# Informe");
    let envelope = envelope_of(&source, &source_artifacts, "job-1");
    let manifest = envelope.report_file.clone().expect("report manifest");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-1", "U", 11, GENERATION_ONE);

    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    assert!(build.pending.is_empty(), "{:?}", build.pending);
    assert_eq!(build.ready.len(), 1);
    let draft = &build.ready[0];
    assert_eq!(draft.job_id, "job-1");
    assert_eq!(draft.op, 'U');
    assert_eq!(draft.changed_at, 11);
    assert_eq!(
        draft.base_seq, 0,
        "base_seq comes only from sync_row_versions"
    );
    let expected: Value =
        serde_json::from_str(&envelope.to_canonical_json().expect("canonical")).expect("value");
    assert_eq!(draft.payload, Some(expected), "the payload is canonical");
    let plan = draft.report_upload.as_ref().expect("upload plan");
    assert_eq!(plan.job_id, "job-1");
    assert_eq!(
        plan.manifest, manifest,
        "the plan carries the proven manifest"
    );
    assert_eq!(draft.acknowledgment.job_id, "job-1");
    assert_eq!(draft.acknowledgment.generation, GENERATION_ONE);

    let change = draft.to_push_change();
    assert_eq!(change.table, ENVELOPE_TABLE);
    assert_eq!(change.row_id, "job-1");
    assert_eq!(change.op, "upsert");
    let wire = serde_json::to_string(&change).expect("wire change");
    assert!(
        !wire.contains(root.to_str().expect("utf-8 root")),
        "no absolute path may enter a wire payload: {wire}"
    );
    assert!(wire.contains(REPORT_FILE_REL_PATH));

    // Building a push never clears the outbox.
    assert!(has_pending_outbox_entry(&sync, "job-1").expect("outbox"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn d_drafts_emit_a_null_payload_without_opening_state() {
    let root = temp_workspace("draft-d");
    let missing_state = root.join("estado-fantasma.sqlite");
    let artifacts_root = root.join("artifacts");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-1", "D", 5, GENERATION_TWO);

    let build = build_push_changes(&sync, &missing_state, &artifacts_root).expect("build");
    assert!(build.pending.is_empty(), "{:?}", build.pending);
    assert_eq!(build.ready.len(), 1);
    let draft = &build.ready[0];
    assert_eq!(draft.job_id, "job-1");
    assert_eq!(draft.op, 'D');
    assert_eq!(draft.changed_at, 5);
    assert_eq!(draft.payload, None, "a tombstone carries a null payload");
    assert!(draft.report_upload.is_none());
    assert_eq!(draft.acknowledgment.generation, GENERATION_TWO);

    let change = draft.to_push_change();
    assert_eq!(change.op, "delete");
    let wire: Value = serde_json::to_value(&change).expect("wire value");
    assert!(
        wire.get("payload").is_none(),
        "a delete change omits the payload entirely"
    );
    assert!(
        !missing_state.exists(),
        "a tombstone never opens or fabricates the research state database"
    );
    assert!(has_pending_outbox_entry(&sync, "job-1").expect("outbox"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn vanished_jobs_and_unreadable_report_files_keep_the_exact_entry_pending() {
    let root = temp_workspace("pending-real");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    // A directory where the managed `report.md` must be: the snapshot is
    // unreadable and the entry stays pending instead of being dropped.
    std::fs::create_dir_all(source_artifacts.join("job-1").join(REPORT_FILE_REL_PATH))
        .expect("report directory");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-fantasma", "U", 7, GENERATION_ONE);
    enqueue_outbox(&sync, "job-1", "U", 8, GENERATION_TWO);

    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    assert!(build.ready.is_empty(), "{:?}", build.ready);
    assert_eq!(
        build.pending,
        outbox_entries(&sync).expect("entries"),
        "the exact outbox entries survive as pending"
    );
    assert!(has_pending_outbox_entry(&sync, "job-fantasma").expect("outbox"));
    assert!(has_pending_outbox_entry(&sync, "job-1").expect("outbox"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_or_changed_report_files_stay_pending_at_proof_time() {
    let root = temp_workspace("pending-proof");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    write_report(&source_artifacts, "job-1", b"# Informe");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-1", "U", 3, GENERATION_ONE);

    // The snapshot captured the manifest, but the file vanished before the
    // build-time proof (the state only a filesystem race can produce).
    let build = build_push_changes_with_report_prover(
        &sync,
        &root.join("source.sqlite"),
        &source_artifacts,
        &|_, _, _| {
            Err(ResearchBlobPending {
                kind: ResearchBlobPendingKind::LocalProofFailed,
                message: "managed report file is missing".to_string(),
            })
        },
    )
    .expect("build");
    assert!(build.ready.is_empty(), "{:?}", build.ready);
    assert_eq!(build.pending, outbox_entries(&sync).expect("entries"));

    // The file changed under the captured manifest hash.
    let build = build_push_changes_with_report_prover(
        &sync,
        &root.join("source.sqlite"),
        &source_artifacts,
        &|_, _, _| {
            Err(ResearchBlobPending {
                kind: ResearchBlobPendingKind::LocalContentChanged,
                message: "report file changed".to_string(),
            })
        },
    )
    .expect("build");
    assert!(build.ready.is_empty(), "{:?}", build.ready);
    assert_eq!(build.pending, outbox_entries(&sync).expect("entries"));

    // The same entry turns ready once the proof passes again.
    let build = build_push_changes_with_report_prover(
        &sync,
        &root.join("source.sqlite"),
        &source_artifacts,
        &|_, job_id, manifest| {
            Ok(ResearchReportUploadPlan {
                job_id: job_id.to_string(),
                manifest: manifest.clone(),
            })
        },
    )
    .expect("build");
    assert_eq!(build.ready.len(), 1);
    assert!(build.pending.is_empty(), "{:?}", build.pending);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn base_seq_comes_only_from_recorded_row_versions() {
    let root = temp_workspace("base-seq");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-1", "U", 3, GENERATION_ONE);

    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    assert_eq!(build.ready[0].base_seq, 0, "never seen: base_seq 0");

    sync.execute(
        "INSERT INTO sync_row_versions(table_name, row_id, server_seq) VALUES (?1, ?2, 41)",
        rusqlite::params![ENVELOPE_TABLE, "job-1"],
    )
    .expect("row version");

    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    assert_eq!(build.ready[0].base_seq, 41);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn applied_settlement_acknowledges_only_the_exact_captured_generation() {
    let root = temp_workspace("settle-applied");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-1", "U", 3, GENERATION_ONE);
    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    let draft = build.ready.into_iter().next().expect("draft");

    // Only applied/lww_won settle anything.
    let mut rejected = applied_result("job-1", 9);
    rejected.status = "lww_lost".to_string();
    let error = settle_applied_push_draft(&sync, &draft, &rejected)
        .expect_err("lww_lost never settles here");
    assert!(
        error.message.contains("applied/lww_won"),
        "{}",
        error.message
    );

    // Invalid and mismatched results are rejected before any bookkeeping.
    let invalid = PushResult {
        server_seq: 0,
        ..applied_result("job-1", 0)
    };
    let error =
        settle_applied_push_draft(&sync, &draft, &invalid).expect_err("invalid server sequence");
    assert!(error.message.contains("server_seq"), "{}", error.message);
    settle_applied_push_draft(&sync, &draft, &applied_result("job-otra", 9))
        .expect_err("mismatched row");
    assert!(
        has_pending_outbox_entry(&sync, "job-1").expect("outbox"),
        "rejected results never touch the outbox"
    );

    // The exact captured generation settles and records the row version.
    let settlement =
        settle_applied_push_draft(&sync, &draft, &applied_result("job-1", 9)).expect("settlement");
    assert_eq!(settlement, PushDraftSettlement::Acknowledged);
    assert!(!has_pending_outbox_entry(&sync, "job-1").expect("outbox"));
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(9));

    // Replaying the settled draft is idempotent.
    let settlement =
        settle_applied_push_draft(&sync, &draft, &applied_result("job-1", 9)).expect("replay");
    assert_eq!(settlement, PushDraftSettlement::AlreadySettled);
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(9));

    // A stale server sequence is rejected and never touches the outbox.
    enqueue_outbox(&sync, "job-1", "U", 4, GENERATION_TWO);
    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    let newer = build.ready.into_iter().next().expect("draft");
    let settlement =
        settle_applied_push_draft(&sync, &newer, &applied_result("job-1", 3)).expect("stale");
    assert_eq!(
        settlement,
        PushDraftSettlement::StaleServerSequence {
            recorded_server_seq: 9,
            response_server_seq: 3,
        }
    );
    assert!(has_pending_outbox_entry(&sync, "job-1").expect("outbox"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn settlement_racing_a_newer_generation_preserves_the_newer_work() {
    let root = temp_workspace("settle-race");
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");

    let sync = sync_connection();
    enqueue_outbox(&sync, "job-1", "U", 3, GENERATION_ONE);
    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    let stale_draft = build.ready.into_iter().next().expect("draft");

    // A save during the send round trip coalesces a fresh generation.
    enqueue_outbox(&sync, "job-1", "U", 4, GENERATION_TWO);

    let settlement = settle_applied_push_draft(&sync, &stale_draft, &applied_result("job-1", 9))
        .expect("race settlement");
    assert_eq!(settlement, PushDraftSettlement::NewerGenerationPending);
    let entries = outbox_entries(&sync).expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].generation, GENERATION_TWO,
        "the newer generation survives"
    );
    assert_eq!(
        recorded_server_seq(&sync, "job-1"),
        Some(9),
        "the server row version still advances"
    );

    // The newer generation is settled by its own push.
    let build =
        build_push_changes(&sync, &root.join("source.sqlite"), &source_artifacts).expect("build");
    let draft = build.ready.into_iter().next().expect("draft");
    assert_eq!(draft.acknowledgment.generation, GENERATION_TWO);
    let settlement =
        settle_applied_push_draft(&sync, &draft, &applied_result("job-1", 10)).expect("settlement");
    assert_eq!(settlement, PushDraftSettlement::Acknowledged);
    assert!(!has_pending_outbox_entry(&sync, "job-1").expect("outbox"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn wire_rows_from_a_foreign_table_never_reach_the_research_machinery() {
    let row = PullRow {
        table: "writing_envelopes".to_string(),
        row_id: "job-1".to_string(),
        server_seq: 6,
        deleted: false,
        changed_at: 6_000,
        device_id: "device-other".to_string(),
        payload: None,
    };
    let error = PulledResearchRow::from_wire_row(&row).expect_err("foreign table");
    assert!(
        error.message.contains("research_envelopes"),
        "{}",
        error.message
    );
}

/// Seeds one local terminal projection plus one pending `U` generation that
/// pushed `local` and is about to lose LWW against `winner`.
fn seeded_lww_fixture(
    name: &str,
) -> (
    PathBuf,
    Connection,
    Connection,
    PathBuf,
    ResearchEnvelopeV1,
    ResearchEnvelopeV1,
) {
    let root = temp_workspace(name);
    let source = research_state(&root.join("source.sqlite"));
    let source_artifacts = root.join("source-artifacts");
    seed_full_job(&source, "job-1");
    let local = envelope_of(&source, &source_artifacts, "job-1");
    bump_report(&source, "job-1");
    let winner = envelope_of(&source, &source_artifacts, "job-1");

    let sync = sync_connection();
    let target = research_state(&root.join("target.sqlite"));
    let target_artifacts = root.join("target-artifacts");
    apply_pulled_row(
        &sync,
        &target,
        &target_artifacts,
        "device-own",
        &upsert_row("job-1", 5, &local),
    )
    .expect("seed target");
    enqueue_outbox(&sync, "job-1", "U", 7, GENERATION_ONE);
    (root, sync, target, target_artifacts, local, winner)
}

#[test]
fn lww_lost_winner_settlement_preserves_the_loser_and_clears_the_exact_generation() {
    let (root, sync, target, target_artifacts, local, winner) = seeded_lww_fixture("lww-lost");
    let ack = outbox_entries(&sync).expect("entries")[0]
        .acknowledgment
        .clone();
    let loser = envelope_of(&target, &target_artifacts, "job-1");
    assert_eq!(loser, local, "the losing side is the seeded local envelope");

    let winner_row =
        PulledResearchRow::from_wire_row(&winner_pull_row("job-1", 6, &winner)).expect("wire row");
    let settlement =
        apply_lww_lost_winner(&sync, &target, &target_artifacts, &winner_row, Some(&ack))
            .expect("settlement");
    let conflict_id = match settlement {
        LwwLostSettlement::Routed(PullApplyOutcome::Applied {
            created: false,
            conflict_id: Some(conflict_id),
        }) => conflict_id,
        other => panic!("expected Applied with a conflict, got {other:?}"),
    };
    assert_eq!(
        conflict_id,
        format!(
            "{RESEARCH_CONFLICT_DIVERGENT_UPSERT}-job-1-{}",
            loser.fingerprint_sha256().expect("fingerprint")
        )
    );

    // The losing local envelope survives in the generic conflict journal.
    let rows = conflicts(&sync);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].3, RESEARCH_CONFLICT_DIVERGENT_UPSERT);
    assert_eq!(
        rows[0].4,
        local.to_canonical_json().expect("canonical loser"),
        "the loser payload is the local envelope"
    );

    // The winner landed and ONLY the adjudicated generation was cleared.
    assert_eq!(envelope_of(&target, &target_artifacts, "job-1"), winner);
    assert!(!has_pending_outbox_entry(&sync, "job-1").expect("outbox"));
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(6));

    // Replaying the settled winner is a stale no-op that journals nothing.
    let settlement =
        apply_lww_lost_winner(&sync, &target, &target_artifacts, &winner_row, Some(&ack))
            .expect("replay");
    assert_eq!(
        settlement,
        LwwLostSettlement::Routed(PullApplyOutcome::Stale)
    );
    assert_eq!(conflicts(&sync).len(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lww_lost_winner_leaves_a_newer_generation_pending() {
    let (root, sync, target, target_artifacts, local, winner) = seeded_lww_fixture("lww-newer");
    let ack = outbox_entries(&sync).expect("entries")[0]
        .acknowledgment
        .clone();
    // A save during the round trip replaced the adjudicated generation.
    enqueue_outbox(&sync, "job-1", "U", 8, GENERATION_TWO);

    let winner_row =
        PulledResearchRow::from_wire_row(&winner_pull_row("job-1", 6, &winner)).expect("wire row");
    let settlement =
        apply_lww_lost_winner(&sync, &target, &target_artifacts, &winner_row, Some(&ack))
            .expect("settlement");
    assert_eq!(settlement, LwwLostSettlement::NewerGenerationPending);

    // The newer generation still owns the row: nothing is applied, journaled
    // or cleared by a wire timestamp.
    assert_eq!(envelope_of(&target, &target_artifacts, "job-1"), local);
    assert!(conflicts(&sync).is_empty(), "nothing was adjudicated yet");
    let entries = outbox_entries(&sync).expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].generation, GENERATION_TWO);
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(5));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lww_lost_winner_clears_nothing_when_the_winner_application_defers() {
    let (root, sync, target, target_artifacts, _local, winner) = seeded_lww_fixture("lww-defer");
    // The guarded local snapshot is invalid: the receive defers instead of
    // reaching an accepted terminal outcome.
    target
        .execute(
            "UPDATE artifacts SET content_json = 'no-es-json'
              WHERE job_id = 'job-1' AND tipo = 'report'",
            [],
        )
        .expect("corrupt local report");
    let ack = outbox_entries(&sync).expect("entries")[0]
        .acknowledgment
        .clone();

    let winner_row =
        PulledResearchRow::from_wire_row(&winner_pull_row("job-1", 6, &winner)).expect("wire row");
    let settlement =
        apply_lww_lost_winner(&sync, &target, &target_artifacts, &winner_row, Some(&ack))
            .expect("settlement");
    match settlement {
        LwwLostSettlement::Routed(PullApplyOutcome::Deferred { .. }) => {}
        other => panic!("expected Deferred, got {other:?}"),
    }
    assert!(
        has_pending_outbox_entry(&sync, "job-1").expect("outbox"),
        "only an accepted terminal outcome may clear the adjudicated generation"
    );
    assert!(conflicts(&sync).is_empty());
    assert_eq!(recorded_server_seq(&sync, "job-1"), Some(5));

    let _ = std::fs::remove_dir_all(&root);
}
