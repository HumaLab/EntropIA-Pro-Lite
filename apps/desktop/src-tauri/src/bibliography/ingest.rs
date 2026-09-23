//! Durable ingest pending tray (E5a): every link/create/upload decision
//! is one row with an idempotency key, a small state machine, and a
//! receipt. Transport never runs without a queued row, and processing
//! demand (E5c) never starts without a receipt carrying verified Zotero
//! identity (item key + version). Pure state machine — the Zotero writes
//! themselves live behind the transport trait (E5b).

use rusqlite::{Connection, OptionalExtension as _};

use super::repository::{BibliographyError, BibliographyResult};
use crate::processing::repository::now_ms;

/// Operation kinds, matching the `kind` CHECK in migration 0054.
pub const KIND_LINK_MATCH: &str = "link_match";
pub const KIND_CREATE_PARENT: &str = "create_parent";
pub const KIND_UPLOAD_ATTACHMENT: &str = "upload_attachment";

/// Operation states, matching the `state` CHECK in migration 0054.
pub const STATE_QUEUED: &str = "queued";
pub const STATE_RUNNING: &str = "running";
pub const STATE_BLOCKED: &str = "blocked";
pub const STATE_SUCCEEDED: &str = "succeeded";
pub const STATE_FAILED: &str = "failed";
pub const STATE_CANCELLED: &str = "cancelled";

/// One explicit user decision: link an existing work, create a parent, or
/// upload an attachment — in exactly one library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestDecision {
    pub kind: String,
    pub library_id: String,
    /// JSON: `{ mode, item_id? }` for links, the item draft for creates,
    /// the file reference for uploads.
    pub payload_json: String,
}

/// One durable tray row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestOperation {
    pub id: String,
    pub request_id: String,
    pub kind: String,
    pub library_id: String,
    pub state: String,
    pub attempt_count: i64,
    pub receipt_json: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
}

/// Records one decision. The same `request_id` returns the existing row
/// unchanged (double-clicks queue exactly one operation); a different
/// decision under a reused `request_id` fails `duplicate_request`.
pub fn record_ingest_decision(
    conn: &Connection,
    request_id: &str,
    decision: &IngestDecision,
) -> BibliographyResult<IngestOperation> {
    if !matches!(
        decision.kind.as_str(),
        KIND_LINK_MATCH | KIND_CREATE_PARENT | KIND_UPLOAD_ATTACHMENT
    ) {
        return Err(BibliographyError::new(
            "invalid_kind",
            "Unknown ingest kind",
        ));
    }
    if let Some(existing) = find_by_request(conn, request_id)? {
        if existing.kind != decision.kind || existing.library_id != decision.library_id {
            return Err(BibliographyError::new(
                "duplicate_request",
                "request_id is already queued with a different decision",
            ));
        }
        return Ok(existing);
    }
    let now = clock_ms();
    let id = uuid::Uuid::new_v4().to_string();
    // A concurrent insert under the same request wins; the loser reads
    // the winner instead of duplicating the tray row.
    match conn.execute(
        "INSERT INTO bibliographic_ingest_operations
             (id, request_id, kind, library_id, payload_json, state,
              attempt_count, receipt_json, last_error_code, last_error_message,
              created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'queued', 0, NULL, NULL, NULL, ?6, ?6)",
        rusqlite::params![
            id,
            request_id,
            decision.kind,
            decision.library_id,
            decision.payload_json,
            now
        ],
    ) {
        Ok(_) => {}
        Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
            if let Some(existing) = find_by_request(conn, request_id)? {
                if existing.kind != decision.kind || existing.library_id != decision.library_id {
                    return Err(BibliographyError::new(
                        "duplicate_request",
                        "request_id is already queued with a different decision",
                    ));
                }
                return Ok(existing);
            }
            return Err(BibliographyError::new(
                "duplicate_request",
                "request_id is already queued",
            ));
        }
        Err(error) => {
            return Err(BibliographyError::new(
                "sql_error",
                format!("Failed to record ingest decision: {error}"),
            ));
        }
    }
    find_by_request(conn, request_id)?
        .ok_or_else(|| BibliographyError::new("sql_error", "Ingest row vanished after insert"))
}

/// Cancels one operation from `queued`, `running`, or `blocked`. Terminal
/// rows fail `invalid_transition`; cancellation never deletes the row.
pub fn cancel_ingest_operation(
    conn: &Connection,
    operation_id: &str,
) -> BibliographyResult<IngestOperation> {
    transition(
        conn,
        operation_id,
        &[STATE_QUEUED, STATE_RUNNING, STATE_BLOCKED],
        STATE_CANCELLED,
        None,
    )
}

/// Requeues one `failed` or `blocked` operation. Anything else fails
/// `invalid_transition`.
pub fn retry_ingest_operation(
    conn: &Connection,
    operation_id: &str,
) -> BibliographyResult<IngestOperation> {
    transition(
        conn,
        operation_id,
        &[STATE_FAILED, STATE_BLOCKED],
        STATE_QUEUED,
        None,
    )
}

/// Marks one `running` operation terminal. `terminal = true` records
/// `failed`, `false` records `blocked` (resumable). Receipts only land
/// via [`complete_ingest_operation`], never through failure.
pub fn fail_ingest_operation(
    conn: &Connection,
    operation_id: &str,
    terminal: bool,
    code: &str,
    message: &str,
) -> BibliographyResult<IngestOperation> {
    let target = if terminal {
        STATE_FAILED
    } else {
        STATE_BLOCKED
    };
    transition(
        conn,
        operation_id,
        &[STATE_QUEUED, STATE_RUNNING],
        target,
        Some((code, message)),
    )
}

/// Records verified Zotero completion: `running` → `succeeded` with the
/// receipt carrying item key + version. E5c gates processing demand on
/// exactly this receipt.
pub fn complete_ingest_operation(
    conn: &Connection,
    operation_id: &str,
    receipt_json: &str,
) -> BibliographyResult<IngestOperation> {
    let operation = require_operation(conn, operation_id)?;
    if operation.state != STATE_RUNNING {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!(
                "Cannot complete an ingest operation from state {}",
                operation.state
            ),
        ));
    }
    conn.execute(
        "UPDATE bibliographic_ingest_operations
         SET state = 'succeeded', receipt_json = ?2,
             last_error_code = NULL, last_error_message = NULL, updated_at = ?3
         WHERE id = ?1",
        rusqlite::params![operation_id, receipt_json, clock_ms()],
    )
    .map_err(|error| {
        BibliographyError::new("sql_error", format!("Failed to complete ingest: {error}"))
    })?;
    require_operation(conn, operation_id)
}

/// Crash recovery: parks every `running` row back to `queued` and returns
/// how many moved. Safe because every transport op is idempotent by
/// request or by key.
pub fn recover_ingest_operations(conn: &Connection) -> BibliographyResult<usize> {
    conn.execute(
        "UPDATE bibliographic_ingest_operations
         SET state = 'queued', updated_at = ?1
         WHERE state = 'running'",
        [clock_ms()],
    )
    .map(|moved| moved as usize)
    .map_err(|error| {
        BibliographyError::new("sql_error", format!("Failed to recover ingest: {error}"))
    })
}

/// Reads one row by id.
pub fn get_ingest_operation(
    conn: &Connection,
    operation_id: &str,
) -> BibliographyResult<Option<IngestOperation>> {
    read_operation(conn, operation_id)
}

fn read_operation(
    conn: &Connection,
    operation_id: &str,
) -> BibliographyResult<Option<IngestOperation>> {
    conn.query_row(
        "SELECT id, request_id, kind, library_id, state, attempt_count,
                receipt_json, last_error_code, last_error_message
         FROM bibliographic_ingest_operations WHERE id = ?1",
        [operation_id],
        |row| {
            Ok(IngestOperation {
                id: row.get(0)?,
                request_id: row.get(1)?,
                kind: row.get(2)?,
                library_id: row.get(3)?,
                state: row.get(4)?,
                attempt_count: row.get(5)?,
                receipt_json: row.get(6)?,
                last_error_code: row.get(7)?,
                last_error_message: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|error| BibliographyError::new("sql_error", format!("Failed to read ingest: {error}")))
}

fn find_by_request(
    conn: &Connection,
    request_id: &str,
) -> BibliographyResult<Option<IngestOperation>> {
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM bibliographic_ingest_operations WHERE request_id = ?1",
            [request_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new("sql_error", format!("Failed to read ingest: {error}"))
        })?;
    id.map(|id| require_operation(conn, &id)).transpose()
}

fn require_operation(conn: &Connection, operation_id: &str) -> BibliographyResult<IngestOperation> {
    read_operation(conn, operation_id)?.ok_or_else(|| {
        BibliographyError::new("unknown_operation", "No ingest operation with that id")
    })
}

fn transition(
    conn: &Connection,
    operation_id: &str,
    allowed: &[&str],
    target: &str,
    failure: Option<(&str, &str)>,
) -> BibliographyResult<IngestOperation> {
    let operation = require_operation(conn, operation_id)?;
    if !allowed.contains(&operation.state.as_str()) {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!(
                "Cannot move an ingest operation from {} to {target}",
                operation.state
            ),
        ));
    }
    match failure {
        Some((code, message)) => conn.execute(
            "UPDATE bibliographic_ingest_operations
             SET state = ?2, last_error_code = ?3, last_error_message = ?4, updated_at = ?5
             WHERE id = ?1",
            rusqlite::params![operation_id, target, code, message, clock_ms()],
        ),
        None => conn.execute(
            "UPDATE bibliographic_ingest_operations
             SET state = ?2, updated_at = ?3
             WHERE id = ?1",
            rusqlite::params![operation_id, target, clock_ms()],
        ),
    }
    .map_err(|error| {
        BibliographyError::new("sql_error", format!("Failed to move ingest: {error}"))
    })?;
    require_operation(conn, operation_id)
}

/// Verified Zotero identity returned by the transport. Processing demand
/// (E5c) starts only from one of these — never from the request payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReceipt {
    pub item_key: String,
    pub version: u64,
    pub library_external_id: String,
}

/// What one transport execution produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// A new parent record now exists in Zotero.
    Created(IngestReceipt),
    /// The payload matched an existing record — linked, never duplicated.
    Linked(IngestReceipt),
    /// The kind has no verified write path in this build (e.g. file
    /// upload through the local connector). Explicit, never faked.
    Unsupported { reason: String },
}

/// Why one transport execution failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestFailure {
    /// `true` records `failed`, `false` records `blocked` (resumable).
    pub terminal: bool,
    pub code: String,
    pub message: String,
}

/// The Zotero write boundary. Fakes cover unit tests; the local-connector
/// transport covers live runs (E5b). Executors never touch HTTP.
pub trait IngestTransport {
    fn execute(&self, operation: &IngestOperation) -> Result<IngestOutcome, IngestFailure>;
}

/// Claims one `queued` operation for execution: `queued` → `running` with
/// `attempt_count + 1`. Anything else fails `invalid_transition`.
pub fn claim_ingest_operation(
    conn: &Connection,
    operation_id: &str,
) -> BibliographyResult<IngestOperation> {
    let operation = require_operation(conn, operation_id)?;
    if operation.state != STATE_QUEUED {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!(
                "Cannot claim an ingest operation from state {}",
                operation.state
            ),
        ));
    }
    conn.execute(
        "UPDATE bibliographic_ingest_operations
         SET state = 'running', attempt_count = attempt_count + 1, updated_at = ?2
         WHERE id = ?1",
        rusqlite::params![operation_id, clock_ms()],
    )
    .map_err(|error| {
        BibliographyError::new("sql_error", format!("Failed to claim ingest: {error}"))
    })?;
    require_operation(conn, operation_id)
}

/// Runs one operation end to end: claim, transport, then complete or fail.
/// A receipt is recorded only from a `Created`/`Linked` outcome while the
/// row is still `running`. If the row left `running` mid-flight (cancel),
/// the terminal state stands but the receipt is still stashed for audit —
/// a Zotero write that happened is never silently dropped.
pub fn run_ingest_operation(
    conn: &Connection,
    operation_id: &str,
    transport: &dyn IngestTransport,
) -> BibliographyResult<IngestOperation> {
    let claimed = claim_ingest_operation(conn, operation_id)?;
    let outcome = transport.execute(&claimed);
    // The row may have left `running` while the transport wrote
    // (cancel). Re-read before deciding what to record.
    let current = require_operation(conn, operation_id)?;
    match outcome {
        Ok(IngestOutcome::Created(receipt)) | Ok(IngestOutcome::Linked(receipt)) => {
            let receipt_json = receipt_to_json(&receipt);
            if current.state == STATE_RUNNING {
                complete_ingest_operation(conn, operation_id, &receipt_json)
            } else {
                stash_receipt(conn, operation_id, &receipt_json)
            }
        }
        Ok(IngestOutcome::Unsupported { reason }) => {
            if current.state == STATE_RUNNING {
                fail_ingest_operation(
                    conn,
                    operation_id,
                    false,
                    "unsupported_kind",
                    &format!("Sin ruta de escritura verificada: {reason}"),
                )
            } else {
                stash_failure(conn, operation_id, "unsupported_kind", &reason)
            }
        }
        Err(failure) => {
            if current.state == STATE_RUNNING {
                fail_ingest_operation(
                    conn,
                    operation_id,
                    failure.terminal,
                    &failure.code,
                    &failure.message,
                )
            } else {
                stash_failure(conn, operation_id, &failure.code, &failure.message)
            }
        }
    }
}

fn receipt_to_json(receipt: &IngestReceipt) -> String {
    serde_json::json!({
        "item_key": receipt.item_key,
        "version": receipt.version,
        "library": receipt.library_external_id,
    })
    .to_string()
}

fn stash_receipt(
    conn: &Connection,
    operation_id: &str,
    receipt_json: &str,
) -> BibliographyResult<IngestOperation> {
    conn.execute(
        "UPDATE bibliographic_ingest_operations
         SET receipt_json = ?2, updated_at = ?3
         WHERE id = ?1",
        rusqlite::params![operation_id, receipt_json, clock_ms()],
    )
    .map_err(|error| {
        BibliographyError::new("sql_error", format!("Failed to stash receipt: {error}"))
    })?;
    require_operation(conn, operation_id)
}

fn stash_failure(
    conn: &Connection,
    operation_id: &str,
    code: &str,
    message: &str,
) -> BibliographyResult<IngestOperation> {
    conn.execute(
        "UPDATE bibliographic_ingest_operations
         SET last_error_code = ?2, last_error_message = ?3, updated_at = ?4
         WHERE id = ?1",
        rusqlite::params![operation_id, code, message, clock_ms()],
    )
    .map_err(|error| {
        BibliographyError::new("sql_error", format!("Failed to stash failure: {error}"))
    })?;
    require_operation(conn, operation_id)
}

#[allow(dead_code)]
fn clock_ms() -> i64 {
    now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tray_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0038_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0054_bibliographic_ingest_operations.sql"
        ))
        .expect("apply tray");
        conn
    }

    fn seed_library(conn: &mut Connection) -> String {
        use super::super::repository::{
            upsert_connection, upsert_library, LibraryType, SourceOrigin, UpsertConnection,
            UpsertLibrary,
        };
        let source = upsert_connection(
            conn,
            UpsertConnection {
                id: "conn-1".to_string(),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        upsert_library(
            conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::User,
                library_id: "0".to_string(),
                name: "Personal".to_string(),
                last_modified_version: Some(1),
            },
        )
        .expect("library")
        .id
    }

    fn link_decision(library_id: &str) -> IngestDecision {
        IngestDecision {
            kind: KIND_LINK_MATCH.to_string(),
            library_id: library_id.to_string(),
            payload_json: r#"{"mode":"link","item_id":"item-1"}"#.to_string(),
        }
    }

    #[test]
    fn decisions_are_idempotent_by_request() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let first =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        assert_eq!(first.state, STATE_QUEUED);
        assert_eq!(first.attempt_count, 0);
        let second = record_ingest_decision(&conn, "req-1", &link_decision(&library_id))
            .expect("re-record the same request");
        assert_eq!(first.id, second.id, "one request means one row");
        assert_eq!(
            second.state, STATE_QUEUED,
            "re-record never resets progress"
        );
    }

    #[test]
    fn reused_requests_with_other_decisions_fail() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        let other = IngestDecision {
            kind: KIND_CREATE_PARENT.to_string(),
            library_id: library_id.clone(),
            payload_json: r#"{"mode":"create","title":"Otro"}"#.to_string(),
        };
        let error = record_ingest_decision(&conn, "req-1", &other).expect_err("must fail");
        assert_eq!(error.code, "duplicate_request");
    }

    #[test]
    fn cancel_never_deletes_and_rejects_terminal_rows() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        let cancelled = cancel_ingest_operation(&conn, &op.id).expect("cancel queued");
        assert_eq!(cancelled.state, STATE_CANCELLED);
        assert!(get_ingest_operation(&conn, &op.id).expect("read").is_some());
        let error = cancel_ingest_operation(&conn, &op.id).expect_err("cancel twice");
        assert_eq!(error.code, "invalid_transition");
    }

    #[test]
    fn retry_requeues_only_failed_or_blocked() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        fail_ingest_operation(&conn, &op.id, false, "zotero_offline", "sin conexión")
            .expect("block");
        let retried = retry_ingest_operation(&conn, &op.id).expect("retry blocked");
        assert_eq!(retried.state, STATE_QUEUED);
        let error = retry_ingest_operation(&conn, &op.id).expect_err("retry queued");
        assert_eq!(error.code, "invalid_transition");
    }

    #[test]
    fn completion_carries_the_verified_receipt() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        conn.execute(
            "UPDATE bibliographic_ingest_operations SET state = 'running' WHERE id = ?1",
            [&op.id],
        )
        .expect("start");
        let done = complete_ingest_operation(
            &conn,
            &op.id,
            r#"{"item_key":"ABC12345","version":3,"library":"0"}"#,
        )
        .expect("complete");
        assert_eq!(done.state, STATE_SUCCEEDED);
        assert!(done.receipt_json.as_deref().unwrap().contains("ABC12345"));
    }

    struct StubTransport {
        outcome: Result<IngestOutcome, IngestFailure>,
        cancel_mid_flight: bool,
    }

    impl IngestTransport for StubTransport {
        fn execute(&self, operation: &IngestOperation) -> Result<IngestOutcome, IngestFailure> {
            if self.cancel_mid_flight {
                // Simulate the user cancelling while Zotero is writing.
                let conn = MidFlightDb::conn();
                cancel_ingest_operation(&conn, &operation.id).expect("cancel mid-flight");
            }
            match &self.outcome {
                Ok(outcome) => Ok(outcome.clone()),
                Err(failure) => Err(failure.clone()),
            }
        }
    }

    // The stub needs the same in-memory database the test drives. Tests
    // run single-threaded here, so a thread-local handle is enough.
    use std::cell::RefCell;
    thread_local! {
        static MID_FLIGHT: RefCell<*const Connection> = RefCell::new(std::ptr::null());
    }
    struct MidFlightDb;
    impl MidFlightDb {
        fn conn() -> &'static Connection {
            MID_FLIGHT.with(|slot| unsafe { &**slot.borrow() })
        }
        fn set(conn: &Connection) {
            MID_FLIGHT.with(|slot| *slot.borrow_mut() = conn as *const Connection);
        }
    }

    fn receipt() -> IngestReceipt {
        IngestReceipt {
            item_key: "ABC12345".to_string(),
            version: 3,
            library_external_id: "0".to_string(),
        }
    }

    #[test]
    fn claim_starts_one_attempt() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        let claimed = claim_ingest_operation(&conn, &op.id).expect("claim");
        assert_eq!(claimed.state, STATE_RUNNING);
        assert_eq!(claimed.attempt_count, 1);
        let error = claim_ingest_operation(&conn, &op.id).expect_err("claim twice");
        assert_eq!(error.code, "invalid_transition");
    }

    #[test]
    fn run_completes_through_the_transport() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        MidFlightDb::set(&conn);
        let done = run_ingest_operation(
            &conn,
            &op.id,
            &StubTransport {
                outcome: Ok(IngestOutcome::Linked(receipt())),
                cancel_mid_flight: false,
            },
        )
        .expect("run");
        assert_eq!(done.state, STATE_SUCCEEDED);
        assert!(done.receipt_json.as_deref().unwrap().contains("ABC12345"));
    }

    #[test]
    fn run_records_transport_failures_with_verdict() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        MidFlightDb::set(&conn);
        let blocked = run_ingest_operation(
            &conn,
            &op.id,
            &StubTransport {
                outcome: Err(IngestFailure {
                    terminal: false,
                    code: "zotero_offline".to_string(),
                    message: "sin conexión".to_string(),
                }),
                cancel_mid_flight: false,
            },
        )
        .expect("run records the failure");
        assert_eq!(blocked.state, STATE_BLOCKED);
        assert_eq!(blocked.last_error_code.as_deref(), Some("zotero_offline"));
    }

    #[test]
    fn cancel_mid_flight_stands_but_keeps_the_receipt() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        MidFlightDb::set(&conn);
        let cancelled = run_ingest_operation(
            &conn,
            &op.id,
            &StubTransport {
                outcome: Ok(IngestOutcome::Created(receipt())),
                cancel_mid_flight: true,
            },
        )
        .expect("run");
        assert_eq!(cancelled.state, STATE_CANCELLED, "the cancel stands");
        assert!(
            cancelled
                .receipt_json
                .as_deref()
                .unwrap()
                .contains("ABC12345"),
            "the write that happened is stashed, never dropped"
        );
    }

    #[test]
    fn unsupported_kinds_block_with_reason() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op = record_ingest_decision(
            &conn,
            "req-1",
            &IngestDecision {
                kind: KIND_UPLOAD_ATTACHMENT.to_string(),
                library_id: library_id.clone(),
                payload_json: r#"{"file":"nota.pdf"}"#.to_string(),
            },
        )
        .expect("record");
        MidFlightDb::set(&conn);
        let blocked = run_ingest_operation(
            &conn,
            &op.id,
            &StubTransport {
                outcome: Ok(IngestOutcome::Unsupported {
                    reason: "no verified file-upload path".to_string(),
                }),
                cancel_mid_flight: false,
            },
        )
        .expect("run");
        assert_eq!(blocked.state, STATE_BLOCKED);
        assert!(blocked
            .last_error_message
            .as_deref()
            .unwrap()
            .contains("no verified"));
    }

    #[test]
    fn recovery_parks_running_back_to_queued() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        conn.execute(
            "UPDATE bibliographic_ingest_operations SET state = 'running' WHERE id = ?1",
            [&op.id],
        )
        .expect("crash mid-run");
        assert_eq!(recover_ingest_operations(&conn).expect("recover"), 1);
        assert_eq!(
            get_ingest_operation(&conn, &op.id)
                .expect("read")
                .expect("row")
                .state,
            STATE_QUEUED
        );
        assert_eq!(recover_ingest_operations(&conn).expect("recover again"), 0);
    }
}
