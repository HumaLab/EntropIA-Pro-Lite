//! Tests for the inactive research envelope snapshot: job selection, strict
//! validation, deterministic source manifest and the bounded `report.md`
//! manifest. Fixtures are temporary `estado.sqlite` files built with the real
//! EntropIA-Agent schema, so the reader is exercised against the columns the
//! engine actually writes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::research_envelope::{
    is_safe_relative_path, report_file_manifest, snapshot_job, snapshot_job_conn,
    verify_report_file, ResearchEnvelopeV1, ResearchFileManifestV1, INVALID_RESEARCH_ENVELOPE,
    MAX_REPORT_FILE_BYTES, RESEARCH_JOB_NOT_FOUND, RESEARCH_JOB_NOT_TERMINAL,
    RESEARCH_REPORT_FILE_MISMATCH, RESEARCH_SNAPSHOT_UNREADABLE,
    UNSUPPORTED_RESEARCH_ENVELOPE_VERSION,
};

fn temp_workspace(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "entropia-research-envelope-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        name
    ));
    std::fs::create_dir_all(&dir).expect("temp workspace");
    dir
}

/// Builds the real `estado.sqlite` schema (EntropIA-Agent owns the DDL) and
/// returns a plain connection for fixture inserts.
fn research_state(path: &Path) -> Connection {
    entropia_agent::estado::EstadoDb::abrir(path.to_str().expect("utf-8 path"))
        .expect("estado schema");
    Connection::open(path).expect("raw state connection")
}

fn insert_job(conn: &Connection, id: &str, modo: &str, status: &str, close_reason: Option<&str>) {
    conn.execute(
        "INSERT INTO jobs (id, modo, pregunta, status, close_reason, config_snapshot,
                           corpus_snapshot_id, project, corpus, created_at, updated_at)
         VALUES (?1, ?2, '¿Pregunta de investigación?', ?3, ?4, '{}', 'snap-1', 'demo',
                 'desktop', 100, 200)",
        rusqlite::params![id, modo, status, close_reason],
    )
    .expect("insert job");
}

fn insert_artifact(
    conn: &Connection,
    job_id: &str,
    id: &str,
    kind: &str,
    version: i64,
    obsolete: bool,
    content: Option<&str>,
) {
    conn.execute(
        "INSERT INTO artifacts (id, job_id, tipo, path, version, created_at, content_json, obsolete)
         VALUES (?1, ?2, ?3, '', ?4, 150, ?5, ?6)",
        rusqlite::params![id, job_id, kind, version, content, obsolete],
    )
    .expect("insert artifact");
}

fn archive_content(evidence: Value) -> String {
    json!({"summary": "síntesis", "claims": [], "limitations": [], "dropped": [], "evidence": evidence})
        .to_string()
}

fn full_fixture(root: &Path) -> PathBuf {
    let state_path = root.join("estado.sqlite");
    let state = research_state(&state_path);
    insert_job(&state, "job-1", "research", "done", Some("completed"));
    insert_artifact(
        &state,
        "job-1",
        "req-1",
        "request",
        1,
        false,
        Some(r#"{"title":"Estudio de campo","question":"¿Pregunta de investigación?"}"#),
    );
    insert_artifact(
        &state,
        "job-1",
        "rep-1",
        "report",
        1,
        true,
        Some(r#"{"report":{"title":"viejo"}}"#),
    );
    insert_artifact(
        &state,
        "job-1",
        "rep-2",
        "report",
        2,
        false,
        Some(r##"{"report":{"title":"vigente"},"markdown":"# Vigente"}"##),
    );
    let evidence = json!([
        {"id":"c1","item_id":"i1","title":"Documento A","text":"texto","start":0,"end":5,
         "asset_id":"a1","collection_id":"col","provenance":"entropia_chunk"},
        {"id":"c1","item_id":"i1","title":"Documento A","text":"texto","start":0,"end":5,
         "asset_id":"a1","collection_id":"col","provenance":"entropia_chunk"},
        {"id":"c1@8000","chunk_id":"c1","item_id":"i1","title":"Documento A","text":"más",
         "start":8000,"end":8004,"provenance":"entropia_chunk"},
        {"id":"c2","item_id":"i2","title":"Documento B","text_hash":"abc123","text":"otro",
         "start":0,"end":5,"provenance":"entropia_chunk"},
        {"id":"zotero:ABCD","title":"Paper Z","provenance":"zotero"}
    ]);
    insert_artifact(
        &state,
        "job-1",
        "arc-1",
        "archive",
        1,
        false,
        Some(&archive_content(evidence)),
    );
    state_path
}

fn write_report_file(root: &Path, job_id: &str, contents: &[u8]) {
    let dir = root.join(job_id);
    std::fs::create_dir_all(&dir).expect("job artifacts dir");
    std::fs::write(dir.join("report.md"), contents).expect("report file");
}

fn error_code(error: &super::research_envelope::ResearchError) -> &str {
    &error.code
}

#[test]
fn snapshot_carries_summary_report_sources_and_file_manifest() {
    let root = temp_workspace("summary");
    let state_path = full_fixture(&root);
    write_report_file(&root, "job-1", b"# Informe\n");

    let envelope = snapshot_job(&state_path, &root, "job-1").expect("terminal snapshot");

    assert_eq!(envelope.id, "job-1");
    assert_eq!(envelope.job.id, "job-1");
    assert_eq!(envelope.job.status, "done");
    assert_eq!(envelope.job.close_reason, "completed");
    assert_eq!(envelope.job.title, "Estudio de campo");
    assert_eq!(envelope.job.question, "¿Pregunta de investigación?");
    assert_eq!(envelope.job.project, "demo");
    assert_eq!(envelope.job.corpus_snapshot_id.as_deref(), Some("snap-1"));
    assert_eq!(envelope.job.created_at, 100);
    assert_eq!(envelope.job.updated_at, 200);

    let report = envelope.report.as_ref().expect("current report");
    assert_eq!(report.version, 2);
    assert_eq!(report.content_json["report"]["title"], "vigente");

    // The duplicate evidence entry collapsed; split parts keep their own id
    // while sharing their source; ordering is deterministic.
    let identities: Vec<(String, String, Option<String>, Option<String>)> = envelope
        .sources
        .iter()
        .map(|entry| {
            (
                entry.source_id.clone(),
                entry.evidence_id.clone(),
                entry.item_id.clone(),
                entry.chunk_id.clone(),
            )
        })
        .collect();
    assert_eq!(
        identities,
        vec![
            (
                "c1".into(),
                "c1".into(),
                Some("i1".into()),
                Some("c1".into())
            ),
            (
                "c1".into(),
                "c1@8000".into(),
                Some("i1".into()),
                Some("c1".into())
            ),
            (
                "c2".into(),
                "c2".into(),
                Some("i2".into()),
                Some("c2".into())
            ),
            ("zotero:ABCD".into(), "zotero:ABCD".into(), None, None),
        ]
    );
    // Available text hash / title fields travel when the evidence carries them.
    assert_eq!(envelope.sources[1].title.as_deref(), Some("Documento A"));
    assert_eq!(envelope.sources[2].text_hash.as_deref(), Some("abc123"));
    assert_eq!(envelope.sources[3].title.as_deref(), Some("Paper Z"));

    let file = envelope.report_file.as_ref().expect("report file manifest");
    assert_eq!(file.rel_path, "report.md");
    assert_eq!(file.size, 10);
    assert_eq!(
        file.sha256,
        format!("{:x}", Sha256::digest(b"# Informe\n".as_slice()))
    );
    verify_report_file(&root, "job-1", file).expect("manifest verifies against the file");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn snapshot_rejects_missing_non_terminal_and_foreign_jobs() {
    let root = temp_workspace("selection");
    let state_path = root.join("estado.sqlite");
    let state = research_state(&state_path);
    insert_job(&state, "job-running", "research", "running", None);
    insert_job(&state, "job-gated", "research", "awaiting_human", None);
    insert_job(&state, "job-paper", "paper", "done", Some("completed"));

    let missing = snapshot_job(&state_path, &root, "job-missing").expect_err("missing job");
    assert_eq!(error_code(&missing), RESEARCH_JOB_NOT_FOUND);

    for id in ["job-running", "job-gated"] {
        let error = snapshot_job(&state_path, &root, id).expect_err("non-terminal job");
        assert_eq!(error_code(&error), RESEARCH_JOB_NOT_TERMINAL);
    }

    let foreign = snapshot_job(&state_path, &root, "job-paper").expect_err("paper job");
    assert_eq!(error_code(&foreign), RESEARCH_JOB_NOT_FOUND);

    let unsafe_id = snapshot_job(&state_path, &root, "../escape").expect_err("unsafe job id");
    assert_eq!(error_code(&unsafe_id), INVALID_RESEARCH_ENVELOPE);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn snapshot_rejects_malformed_required_json() {
    let root = temp_workspace("malformed");
    let state_path = root.join("estado.sqlite");
    let state = research_state(&state_path);

    insert_job(
        &state,
        "job-bad-report",
        "research",
        "done",
        Some("completed"),
    );
    insert_artifact(
        &state,
        "job-bad-report",
        "rep-bad",
        "report",
        1,
        false,
        Some("esto no es JSON"),
    );
    let error = snapshot_job_conn(&state, &root, "job-bad-report").expect_err("bad report json");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    insert_job(
        &state,
        "job-null-report",
        "research",
        "done",
        Some("completed"),
    );
    insert_artifact(
        &state,
        "job-null-report",
        "rep-null",
        "report",
        1,
        false,
        None,
    );
    let error = snapshot_job_conn(&state, &root, "job-null-report").expect_err("null report");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    insert_job(
        &state,
        "job-bad-archive",
        "research",
        "done",
        Some("completed"),
    );
    insert_artifact(
        &state,
        "job-bad-archive",
        "arc-bad",
        "archive",
        1,
        false,
        Some(&archive_content(json!([{"title": "sin id"}]))),
    );
    let error = snapshot_job_conn(&state, &root, "job-bad-archive").expect_err("id-less evidence");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    insert_job(
        &state,
        "job-bad-title",
        "research",
        "failed",
        Some("blocked"),
    );
    insert_artifact(
        &state,
        "job-bad-title",
        "req-bad",
        "request",
        1,
        false,
        Some(r#"{"title": 42}"#),
    );
    let error = snapshot_job_conn(&state, &root, "job-bad-title").expect_err("non-string title");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    insert_job(
        &state,
        "job-conflict",
        "research",
        "done",
        Some("completed"),
    );
    insert_artifact(
        &state,
        "job-conflict",
        "arc-conflict",
        "archive",
        1,
        false,
        Some(&archive_content(json!([
            {"id": "c1", "title": "A"},
            {"id": "c1", "title": "B"}
        ]))),
    );
    let error = snapshot_job_conn(&state, &root, "job-conflict").expect_err("conflicting dup id");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn report_less_failed_job_stays_representable() {
    let root = temp_workspace("report-less");
    let state_path = root.join("estado.sqlite");
    let state = research_state(&state_path);
    insert_job(&state, "job-f", "research", "failed", Some("blocked"));

    let envelope = snapshot_job_conn(&state, &root, "job-f").expect("failed snapshot");
    assert!(envelope.report.is_none());
    assert!(envelope.sources.is_empty());
    assert!(envelope.report_file.is_none());
    assert_eq!(envelope.job.close_reason, "blocked");
    assert_eq!(envelope.job.title, envelope.job.question);

    // Canonical serialization round-trips and the fingerprint is stable.
    let canonical = envelope.to_canonical_json().expect("canonical json");
    let roundtrip = ResearchEnvelopeV1::from_json(&canonical).expect("roundtrip");
    assert_eq!(roundtrip, envelope);
    assert_eq!(
        envelope.fingerprint_sha256().expect("fingerprint"),
        roundtrip.fingerprint_sha256().expect("fingerprint")
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn report_file_manifest_is_optional_and_read_bounded() {
    let root = temp_workspace("files");
    let state_path = root.join("estado.sqlite");
    let state = research_state(&state_path);
    insert_job(&state, "job-1", "research", "done", Some("completed"));
    insert_job(&state, "job-2", "research", "done", Some("cancelled"));

    // Absent file: the manifest is simply None.
    assert!(report_file_manifest(&root, "job-1")
        .expect("absent manifest")
        .is_none());

    write_report_file(&root, "job-1", b"# Informe\n");
    let manifest = report_file_manifest(&root, "job-1")
        .expect("manifest")
        .expect("present");
    assert_eq!(
        manifest,
        ResearchFileManifestV1 {
            rel_path: "report.md".into(),
            sha256: format!("{:x}", Sha256::digest(b"# Informe\n".as_slice())),
            size: 10,
        }
    );

    // A report file over the bound is a hard error, never a truncated hash.
    let dir = root.join("job-2");
    std::fs::create_dir_all(&dir).expect("job-2 dir");
    let big = std::fs::File::create(dir.join("report.md")).expect("big report file");
    big.set_len(MAX_REPORT_FILE_BYTES + 1)
        .expect("oversized file");
    let error = report_file_manifest(&root, "job-2").expect_err("oversized report");
    assert_eq!(error_code(&error), RESEARCH_SNAPSHOT_UNREADABLE);

    // Verification catches any drift after the manifest was built.
    std::fs::write(root.join("job-1").join("report.md"), b"# Otro\n").expect("rewrite");
    let error = verify_report_file(&root, "job-1", &manifest).expect_err("changed report");
    assert_eq!(error_code(&error), RESEARCH_REPORT_FILE_MISMATCH);

    // A directory where the report file should be is never hashed.
    let dir = root.join("job-2");
    std::fs::remove_file(dir.join("report.md")).expect("remove file");
    std::fs::create_dir(dir.join("report.md")).expect("directory in the way");
    let error = report_file_manifest(&root, "job-2").expect_err("directory report");
    assert_eq!(error_code(&error), RESEARCH_SNAPSHOT_UNREADABLE);

    let _ = std::fs::remove_dir_all(&root);
}

fn envelope_json(sources: Value, content: Value) -> String {
    json!({
        "id": "job-1",
        "envelope_version": 1,
        "job": {
            "id": "job-1",
            "status": "done",
            "close_reason": "completed",
            "title": "Título",
            "question": "¿Pregunta?",
            "project": "demo",
            "corpus_snapshot_id": "snap-1",
            "created_at": 100,
            "updated_at": 200
        },
        "report": {"version": 2, "content_json": content},
        "sources": sources,
        "report_file": {"rel_path": "report.md", "sha256": "0".repeat(64), "size": 12}
    })
    .to_string()
}

#[test]
fn canonical_json_and_fingerprint_ignore_aggregate_order() {
    let source_a = json!({"source_id": "c1", "evidence_id": "c1", "title": "A"});
    let source_b = json!({"source_id": "c2", "evidence_id": "c2", "title": "B"});

    let first = ResearchEnvelopeV1::from_json(&envelope_json(
        json!([source_a.clone(), source_b.clone()]),
        json!({"z": 1, "a": {"y": 2, "b": 3}}),
    ))
    .expect("first envelope");
    let second = ResearchEnvelopeV1::from_json(&envelope_json(
        json!([source_b, source_a]),
        json!({"a": {"b": 3, "y": 2}, "z": 1}),
    ))
    .expect("second envelope");

    assert_eq!(
        first.to_canonical_json().expect("canonical"),
        second.to_canonical_json().expect("canonical")
    );
    assert_eq!(
        first.fingerprint_sha256().expect("fingerprint"),
        second.fingerprint_sha256().expect("fingerprint")
    );
}

#[test]
fn from_json_fails_closed_on_unsupported_or_unsafe_payloads() {
    let unsupported = envelope_json(json!([]), json!({}))
        .replace(r#""envelope_version":1"#, r#""envelope_version":2"#);
    let error = ResearchEnvelopeV1::from_json(&unsupported).expect_err("unsupported version");
    assert_eq!(error_code(&error), UNSUPPORTED_RESEARCH_ENVELOPE_VERSION);

    let unknown_field = envelope_json(json!([]), json!({}))
        .replace(r#""sources":"#, r#""surprise":true,"sources":"#);
    let error = ResearchEnvelopeV1::from_json(&unknown_field).expect_err("unknown field");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    let mismatched_id =
        envelope_json(json!([]), json!({})).replacen(r#""id":"job-1","#, r#""id":"job-2","#, 1);
    let error = ResearchEnvelopeV1::from_json(&mismatched_id).expect_err("id mismatch");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    let active =
        envelope_json(json!([]), json!({})).replace(r#""status":"done""#, r#""status":"running""#);
    let error = ResearchEnvelopeV1::from_json(&active).expect_err("non-terminal status");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    // The stored vocabulary is underscore-spelled; the PROTOCOL prose spelling
    // is not silently accepted.
    let hyphenated = envelope_json(json!([]), json!({})).replace(
        r#""close_reason":"completed""#,
        r#""close_reason":"budget-exhausted""#,
    );
    let error = ResearchEnvelopeV1::from_json(&hyphenated).expect_err("unknown close_reason");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    let duplicates = envelope_json(
        json!([
            {"source_id": "c1", "evidence_id": "ev"},
            {"source_id": "c2", "evidence_id": "ev"}
        ]),
        json!({}),
    );
    let error = ResearchEnvelopeV1::from_json(&duplicates).expect_err("duplicate evidence id");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    let unsafe_path = envelope_json(json!([]), json!({}))
        .replace(r#""rel_path":"report.md""#, r#""rel_path":"../report.md""#);
    let error = ResearchEnvelopeV1::from_json(&unsafe_path).expect_err("unsafe rel path");
    assert_eq!(error_code(&error), INVALID_RESEARCH_ENVELOPE);

    assert!(!is_safe_relative_path(".."));
    assert!(!is_safe_relative_path("a/../b"));
    assert!(!is_safe_relative_path("C:/x"));
    assert!(!is_safe_relative_path("a\\b"));
    assert!(is_safe_relative_path("report.md"));
}
