//! Durable ingest pending tray (E5a):
//! every link/create/upload decision
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
    pub payload_json: String,
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
        "SELECT id, request_id, kind, library_id, payload_json, state, attempt_count,
                receipt_json, last_error_code, last_error_message
         FROM bibliographic_ingest_operations WHERE id = ?1",
        [operation_id],
        |row| {
            Ok(IngestOperation {
                id: row.get(0)?,
                request_id: row.get(1)?,
                kind: row.get(2)?,
                library_id: row.get(3)?,
                payload_json: row.get(4)?,
                state: row.get(5)?,
                attempt_count: row.get(6)?,
                receipt_json: row.get(7)?,
                last_error_code: row.get(8)?,
                last_error_message: row.get(9)?,
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

/// Offline-capable transport over the verified catalog (E5b).
///
/// - `link_match` resolves the payload item in the catalog: live rows
///   link, unknown keys fail `unknown_item`, tombstoned rows fail
///   `tombstoned`, rows from another library fail `wrong_library`.
/// - `create_parent` stages the parent locally under a deterministic key
///   derived from the request id: the same request stages exactly one
///   row, and a colliding key with the same title links instead of
///   duplicating. Receipts carry `version: 0`, meaning *staged locally,*
///   *awaiting the Zotero write* — the live `saveItems` round-trip
///   against the isolated test group runs when Zotero is reachable.
/// - `upload_attachment` reports `Unsupported`: the local connector has
///   no verified file-upload path, so there is no fake success.
pub struct CatalogIngestTransport;

impl IngestTransport for CatalogIngestTransport {
    fn execute(
        &self,
        conn: &mut Connection,
        operation: &IngestOperation,
    ) -> Result<IngestOutcome, IngestFailure> {
        match operation.kind.as_str() {
            KIND_LINK_MATCH => link_match(conn, operation),
            KIND_CREATE_PARENT => create_parent(conn, operation),
            KIND_UPLOAD_ATTACHMENT => Ok(IngestOutcome::Unsupported {
                reason: "the local connector has no verified file-upload path".to_string(),
            }),
            kind => Err(IngestFailure {
                terminal: true,
                code: "invalid_kind".to_string(),
                message: format!("Unknown ingest kind {kind}"),
            }),
        }
    }
}

fn terminal(code: &str, message: String) -> IngestFailure {
    IngestFailure {
        terminal: true,
        code: code.to_string(),
        message,
    }
}

fn link_match(
    conn: &Connection,
    operation: &IngestOperation,
) -> Result<IngestOutcome, IngestFailure> {
    let payload: serde_json::Value =
        serde_json::from_str(&operation.payload_json).map_err(|_| {
            terminal(
                "invalid_payload",
                "El enlace no trae un item válido.".to_string(),
            )
        })?;
    let item_id = payload
        .get("item_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            terminal(
                "invalid_payload",
                "El enlace no trae un item válido.".to_string(),
            )
        })?;
    let row: Option<(String, Option<i64>, String, Option<String>)> = conn
        .query_row(
            "SELECT i.item_key, i.item_version, l.library_id,
                    (SELECT t.item_id FROM zotero_item_tombstones t WHERE t.item_id = i.id)
             FROM bibliographic_items i
             JOIN zotero_libraries l ON l.id = i.library_id
             WHERE i.id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| {
            terminal(
                "sql_error",
                format!("No se pudo resolver el enlace: {error}"),
            )
        })?;
    // item_key, item_version, library_external_id, tombstone
    let (item_key, item_version, external, tombstone): (
        String,
        Option<i64>,
        String,
        Option<String>,
    ) = match row {
        None => {
            return Err(terminal(
                "unknown_item",
                "Ese trabajo ya no está en el catálogo.".to_string(),
            ))
        }
        Some((key, version, external, tomb)) => (key, version, external, tomb),
    };
    if tombstone.is_some() {
        return Err(terminal(
            "tombstoned",
            "Ese trabajo fue revocado en Zotero.".to_string(),
        ));
    }
    // The decision names one internal library row; resolve its external
    // namespace and refuse cross-library links instead of misfiling.
    let decided_external: Option<String> = conn
        .query_row(
            "SELECT library_id FROM zotero_libraries WHERE id = ?1",
            [&operation.library_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            terminal(
                "sql_error",
                format!("No se pudo resolver la biblioteca: {error}"),
            )
        })?;
    if decided_external.as_deref() != Some(external.as_str()) {
        return Err(terminal(
            "wrong_library",
            "Ese trabajo pertenece a otra biblioteca.".to_string(),
        ));
    }
    Ok(IngestOutcome::Linked(IngestReceipt {
        item_key,
        version: item_version.unwrap_or(0) as u64,
        library_external_id: external,
    }))
}

fn create_parent(
    conn: &mut Connection,
    operation: &IngestOperation,
) -> Result<IngestOutcome, IngestFailure> {
    let payload: serde_json::Value =
        serde_json::from_str(&operation.payload_json).map_err(|_| {
            terminal(
                "invalid_payload",
                "La creación no trae un título válido.".to_string(),
            )
        })?;
    let title = payload
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or_else(|| {
            terminal(
                "invalid_payload",
                "La creación no trae un título válido.".to_string(),
            )
        })?;
    let key = derive_create_key(&operation.request_id);
    // Idempotent by key: a colliding key with the same title links the
    // existing record; a different title under the same key is a hash
    // collision and fails loudly instead of merging two works.
    let existing: Option<(String, Option<i64>, String, String)> = conn
        .query_row(
            "SELECT i.id, i.item_version, l.library_id, i.title
             FROM bibliographic_items i
             JOIN zotero_libraries l ON l.id = i.library_id
             WHERE i.item_key = ?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| {
            terminal(
                "sql_error",
                format!("No se pudo revisar duplicados: {error}"),
            )
        })?;
    if let Some((_, version, external, existing_title)) = existing {
        if existing_title.trim().eq_ignore_ascii_case(title) {
            return Ok(IngestOutcome::Linked(IngestReceipt {
                item_key: key,
                version: version.unwrap_or(0) as u64,
                library_external_id: external,
            }));
        }
        return Err(terminal(
            "key_collision",
            "La clave derivada ya pertenece a otra obra.".to_string(),
        ));
    }
    let external: String = conn
        .query_row(
            "SELECT library_id FROM zotero_libraries WHERE id = ?1",
            [&operation.library_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            terminal(
                "sql_error",
                format!("No se pudo resolver la biblioteca: {error}"),
            )
        })?
        .ok_or_else(|| terminal("unknown_library", "La biblioteca ya no existe.".to_string()))?;
    let snapshot = serde_json::json!({
        "key": key,
        "title": title,
        "staged_by": "zsb-ingest",
        "request_id": operation.request_id,
    })
    .to_string();
    super::repository::upsert_item(
        conn,
        &operation.library_id,
        super::repository::BibliographicItemInput {
            item_key: key.clone(),
            item_version: None,
            native_json_snapshot: snapshot.clone(),
            csl_json_snapshot: snapshot,
            title: Some(title.to_string()),
            ..Default::default()
        },
    )
    .map_err(|error| {
        terminal(
            "sql_error",
            format!("No se pudo crear el registro: {}", error.code),
        )
    })?;
    Ok(IngestOutcome::Created(IngestReceipt {
        item_key: key,
        version: 0,
        library_external_id: external,
    }))
}

/// Deterministic 8-char `[A-Z0-9]` key from the request id (FNV-1a, 40
/// bits). The same request always stages the same key, so retries and
/// double-clicks converge instead of duplicating.
fn derive_create_key(request_id: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in request_id.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let mut key = String::with_capacity(8);
    let mut bits = hash;
    for _ in 0..8 {
        key.push(ALPHABET[(bits & 31) as usize] as char);
        bits >>= 5;
    }
    key
}

/// The Zotero write boundary. Fakes cover unit tests; the local-connector
/// transport covers live runs (E5b). Executors never touch HTTP.
pub trait IngestTransport {
    fn execute(
        &self,
        conn: &mut Connection,
        operation: &IngestOperation,
    ) -> Result<IngestOutcome, IngestFailure>;
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
    conn: &mut Connection,
    operation_id: &str,
    transport: &dyn IngestTransport,
) -> BibliographyResult<IngestOperation> {
    let claimed = claim_ingest_operation(conn, operation_id)?;
    let outcome = transport.execute(conn, &claimed);
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

/// A succeeded operation whose receipt survived verification against the
/// catalog. Processing demand starts from one of these — never from the
/// request payload, never from a staged (`version: 0`) receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIngestWork {
    pub operation_id: String,
    pub item_id: String,
    pub item_key: String,
    /// Catalog version at verification time (>= the receipt version).
    pub version: i64,
    pub library_row_id: String,
}

/// How many processing demands one verified work admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestAdmission {
    pub profile_demands_admitted: usize,
    pub extraction_demands_admitted: usize,
}

/// Verifies one `succeeded` operation against the catalog (E5c gate):
///
/// - anything but `succeeded` fails `not_completed`;
/// - staged receipts (`version: 0`) fail `not_verified` — the Zotero
///   write has not happened yet, so no processing starts;
/// - unknown keys fail `unknown_item`, revoked works `tombstoned`;
/// - a catalog older than the receipt fails `stale_receipt` — sync has
///   not observed the write yet, so processing waits for sync.
pub fn verify_ingest_receipt(
    conn: &Connection,
    operation_id: &str,
) -> BibliographyResult<VerifiedIngestWork> {
    let operation = require_operation(conn, operation_id)?;
    if operation.state != STATE_SUCCEEDED {
        return Err(BibliographyError::new(
            "not_completed",
            format!(
                "Ingest is {} — processing starts only after verified Zotero completion",
                operation.state
            ),
        ));
    }
    let receipt: serde_json::Value = operation
        .receipt_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .ok_or_else(|| {
            BibliographyError::new("invalid_receipt", "La operación no trae un recibo válido.")
        })?;
    let item_key = receipt
        .get("item_key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            BibliographyError::new("invalid_receipt", "El recibo no trae una clave válida.")
        })?;
    let receipt_version = receipt
        .get("version")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            BibliographyError::new("invalid_receipt", "El recibo no trae una versión válida.")
        })?;
    if receipt_version == 0 {
        return Err(BibliographyError::new(
            "not_verified",
            "El registro aún no existe en Zotero — el procesamiento espera la escritura.",
        ));
    }
    let library_external = receipt
        .get("library")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let row: Option<(String, String, Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT i.id, i.library_id, i.item_version,
                    (SELECT t.item_id FROM zotero_item_tombstones t WHERE t.item_id = i.id)
             FROM bibliographic_items i
             JOIN zotero_libraries l ON l.id = i.library_id
             WHERE i.item_key = ?1 AND l.library_id = ?2",
            rusqlite::params![item_key, library_external],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new("sql_error", format!("Failed to verify receipt: {error}"))
        })?;
    let (item_id, library_row_id, item_version, tombstone) = row.ok_or_else(|| {
        BibliographyError::new("unknown_item", "Ese trabajo ya no está en el catálogo.")
    })?;
    if tombstone.is_some() {
        return Err(BibliographyError::new(
            "tombstoned",
            "Ese trabajo fue revocado en Zotero.",
        ));
    }
    match item_version {
        None => Err(BibliographyError::new(
            "not_verified",
            "El catálogo aún no vio ese registro en Zotero.",
        )),
        Some(version) if (version as u64) < receipt_version => Err(BibliographyError::new(
            "stale_receipt",
            "La sincronización aún no observa esa escritura — el procesamiento espera.",
        )),
        Some(version) => Ok(VerifiedIngestWork {
            operation_id: operation_id.to_string(),
            item_id,
            item_key: item_key.to_string(),
            version,
            library_row_id,
        }),
    }
}

/// Admits profile + extraction demand for one verified work through the
/// existing idempotent chaining. Library-wide and duplicate-safe, so the
/// admitted work converges with every other stale work.
pub fn admit_verified_work(
    conn: &Connection,
    work: &VerifiedIngestWork,
) -> BibliographyResult<IngestAdmission> {
    let profile_demands_admitted =
        crate::processing::repository::admit_stale_profile_demands(conn, &work.library_row_id)
            .map_err(|message| BibliographyError::new("admission_failed", message))?;
    let extraction_demands_admitted =
        crate::processing::repository::admit_stale_extraction_demands(conn, &work.library_row_id)
            .map_err(|message| BibliographyError::new("admission_failed", message))?;
    Ok(IngestAdmission {
        profile_demands_admitted,
        extraction_demands_admitted,
    })
}

/// Live write transport through the local Zotero connector (E5b-live).
///
/// Verified 2026-09-23 against the isolated `prueba` group (`6680944`):
/// `POST /connector/saveItems` with `{items:[...]}` returns `201` and
/// lands in the library currently selected in the client — the group
/// when the group is selected. Writes never touch the personal library:
/// the transport refuses any `library_external_id` other than the one
/// it was constructed with, and construction is the caller's explicit
/// targeting decision (test group only).
///
/// Idempotency across lost responses comes from a marker tag
/// (`zsb-req:{request8}`) baked into every draft: before creating, the
/// transport searches the tag, and a hit returns the existing record as
/// `Created` instead of duplicating it.
pub struct ZoteroConnectorTransport {
    base_url: String,
    library_external_id: String,
}

impl ZoteroConnectorTransport {
    /// Targets exactly one external library namespace (test group only).
    pub fn for_test_group(base_url: &str, library_external_id: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            library_external_id: library_external_id.to_string(),
        }
    }

    fn http(&self) -> Result<reqwest::blocking::Client, IngestFailure> {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|error| IngestFailure {
                terminal: false,
                code: "http_client".to_string(),
                message: format!("No se pudo preparar el cliente Zotero: {error}"),
            })
    }

    fn ping(&self, client: &reqwest::blocking::Client) -> Result<(), IngestFailure> {
        let offline = || IngestFailure {
            terminal: false,
            code: "zotero_offline".to_string(),
            message: "Zotero no responde en esta máquina.".to_string(),
        };
        let response = client
            .get(format!("{}/connector/ping", self.base_url))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .map_err(|_| offline())?;
        let body = response.text().map_err(|_| offline())?;
        if body.contains("Zotero is running") {
            Ok(())
        } else {
            Err(offline())
        }
    }

    /// Reads one created record back by exact title plus the marker tag.
    /// The local `q` scope does not reliably cover tags right after a
    /// write (verified live 2026-09-23), so the title -- unique per
    /// request -- is the lookup key and the tag is the identity check.
    /// One immediate attempt plus one delayed retry absorb index lag;
    /// a persistent miss stays `None` (retry-safe, never a duplicate).
    fn readback(
        &self,
        client: &reqwest::blocking::Client,
        title: &str,
        marker_tag: &str,
    ) -> Result<Option<IngestReceipt>, IngestFailure> {
        for attempt in 0..2 {
            if attempt == 1 {
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            match self.readback_once(client, title, marker_tag)? {
                Some(receipt) => return Ok(Some(receipt)),
                None => continue,
            }
        }
        Ok(None)
    }

    /// Waits for the group sync to observe the write: a fresh local item
    /// reads back `version: 0` until it syncs, and E5c admits no
    /// processing on an unobserved write. Polls bounded (~30s); a
    /// timeout stays `None` so the next attempt links instead of
    /// duplicating.
    fn await_sync(
        &self,
        client: &reqwest::blocking::Client,
        title: &str,
        marker_tag: &str,
    ) -> Result<Option<IngestReceipt>, IngestFailure> {
        for _ in 0..15 {
            match self.readback(client, title, marker_tag)? {
                Some(receipt) if receipt.version >= 1 => return Ok(Some(receipt)),
                _ => std::thread::sleep(std::time::Duration::from_secs(2)),
            }
        }
        Ok(None)
    }

    fn readback_once(
        &self,
        client: &reqwest::blocking::Client,
        title: &str,
        marker_tag: &str,
    ) -> Result<Option<IngestReceipt>, IngestFailure> {
        let response = client
            .get(format!(
                "{}/api/groups/{}/items",
                self.base_url, self.library_external_id
            ))
            .query(&[("q", title), ("format", "json"), ("limit", "25")])
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .map_err(|error| IngestFailure {
                terminal: false,
                code: "readback_failed".to_string(),
                message: format!("No se pudo releer el grupo: {error}"),
            })?;
        if !response.status().is_success() {
            return Err(IngestFailure {
                terminal: false,
                code: "readback_failed".to_string(),
                message: format!("El grupo devolvió {}", response.status()),
            });
        }
        let items: serde_json::Value = response.json().map_err(|error| IngestFailure {
            terminal: false,
            code: "readback_failed".to_string(),
            message: format!("Respuesta ilegible del grupo: {error}"),
        })?;
        let found = items.as_array().into_iter().flatten().find_map(|item| {
            let data = item.get("data")?;
            if data.get("title")?.as_str()? != title {
                return None;
            }
            let marked = data
                .get("tags")?
                .as_array()?
                .iter()
                .any(|tag| tag.get("tag").and_then(|t| t.as_str()) == Some(marker_tag));
            if !marked {
                return None;
            }
            Some(IngestReceipt {
                item_key: item.get("key")?.as_str()?.to_string(),
                version: item.get("version")?.as_u64()?,
                library_external_id: self.library_external_id.clone(),
            })
        });
        Ok(found)
    }

    fn create_live(
        &self,
        client: &reqwest::blocking::Client,
        operation: &IngestOperation,
        title: &str,
        item_type: &str,
    ) -> Result<IngestOutcome, IngestFailure> {
        let marker_tag = format!("zsb-req:{}", &request_fingerprint(&operation.request_id));
        // Lost-response safety first: a previous attempt may have landed.
        if let Some(receipt) = self.readback(client, title, &marker_tag)? {
            return Ok(IngestOutcome::Created(receipt));
        }
        let draft = serde_json::json!([{
            "itemType": item_type,
            "title": title,
            "tags": [{ "tag": marker_tag }],
        }]);
        let response = client
            .post(format!("{}/connector/saveItems", self.base_url))
            .json(&serde_json::json!({ "items": draft }))
            .send()
            .map_err(|error| IngestFailure {
                terminal: false,
                code: "zotero_offline".to_string(),
                message: format!("No se pudo escribir en Zotero: {error}"),
            })?;
        if response.status().as_u16() != 201 {
            return Err(IngestFailure {
                terminal: false,
                code: "zotero_rejected".to_string(),
                message: format!("Zotero devolvió {}", response.status()),
            });
        }
        self.await_sync(client, title, &marker_tag)?
            .map(IngestOutcome::Created)
            .ok_or(IngestFailure {
                terminal: false,
                code: "readback_miss".to_string(),
                message:
                    "Zotero aceptó el registro pero aún no lo devuelve; reintentable sin duplicar."
                        .to_string(),
            })
    }
}

/// First 8 chars of the hex FNV-1a of the request id — stable, short,
/// and safe inside a Zotero tag.
fn request_fingerprint(request_id: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in request_id.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")[..8].to_string()
}

impl IngestTransport for ZoteroConnectorTransport {
    fn execute(
        &self,
        conn: &mut Connection,
        operation: &IngestOperation,
    ) -> Result<IngestOutcome, IngestFailure> {
        if operation.library_id.is_empty() {
            return Err(IngestFailure {
                terminal: true,
                code: "invalid_payload".to_string(),
                message: "La operación no trae biblioteca.".to_string(),
            });
        }
        // The decision names an internal library row; the transport only
        // writes when its external namespace matches the targeted one.
        // Anything else fails before any HTTP happens.
        let decided_external: Option<String> = conn
            .query_row(
                "SELECT library_id FROM zotero_libraries WHERE id = ?1",
                [&operation.library_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| IngestFailure {
                terminal: true,
                code: "sql_error".to_string(),
                message: format!("No se pudo resolver la biblioteca: {error}"),
            })?;
        if decided_external.as_deref() != Some(self.library_external_id.as_str()) {
            return Err(IngestFailure {
                terminal: true,
                code: "wrong_library".to_string(),
                message: "Ese transporte solo escribe en el grupo de prueba.".to_string(),
            });
        }
        match operation.kind.as_str() {
            KIND_LINK_MATCH => CatalogIngestTransport.execute(conn, operation),
            KIND_CREATE_PARENT => {
                let client = self.http()?;
                self.ping(&client)?;
                let payload: serde_json::Value = serde_json::from_str(&operation.payload_json)
                    .map_err(|_| IngestFailure {
                        terminal: true,
                        code: "invalid_payload".to_string(),
                        message: "La creación no trae un título válido.".to_string(),
                    })?;
                let title = payload
                    .get("title")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .ok_or(IngestFailure {
                        terminal: true,
                        code: "invalid_payload".to_string(),
                        message: "La creación no trae un título válido.".to_string(),
                    })?;
                let item_type = payload
                    .get("item_type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("book");
                self.create_live(&client, operation, title, item_type)
            }
            KIND_UPLOAD_ATTACHMENT => Ok(IngestOutcome::Unsupported {
                reason: "the local connector has no verified file-upload path".to_string(),
            }),
            kind => Err(IngestFailure {
                terminal: true,
                code: "invalid_kind".to_string(),
                message: format!("Unknown ingest kind {kind}"),
            }),
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
            "../../../../../packages/store/src/migrations/0039_bibliography_relations.sql"
        ))
        .expect("apply relations");
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
        fn execute(
            &self,
            conn: &mut Connection,
            operation: &IngestOperation,
        ) -> Result<IngestOutcome, IngestFailure> {
            if self.cancel_mid_flight {
                // Simulate the user cancelling while Zotero is writing.
                cancel_ingest_operation(conn, &operation.id).expect("cancel mid-flight");
            }
            match &self.outcome {
                Ok(outcome) => Ok(outcome.clone()),
                Err(failure) => Err(failure.clone()),
            }
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
        let done = run_ingest_operation(
            &mut conn,
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
        let blocked = run_ingest_operation(
            &mut conn,
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
        let cancelled = run_ingest_operation(
            &mut conn,
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
        let blocked = run_ingest_operation(
            &mut conn,
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

    fn catalog_db() -> (Connection, String, String) {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        use super::super::repository::{upsert_item, BibliographicItemInput};
        let item = upsert_item(
            &mut conn,
            &library_id,
            BibliographicItemInput {
                item_key: "LIVE0001".to_string(),
                item_version: Some(3),
                native_json_snapshot: r#"{"key":"LIVE0001","version":3}"#.to_string(),
                csl_json_snapshot: r#"{"id":"LIVE0001","title":"Obra viva"}"#.to_string(),
                title: Some("Obra viva".to_string()),
                ..Default::default()
            },
        )
        .expect("seed work");
        (conn, library_id, item.id)
    }

    #[test]
    fn link_resolves_a_live_work() {
        let (mut conn, library_id, item_id) = catalog_db();
        let transport = CatalogIngestTransport;
        let op = record_ingest_decision(
            &conn,
            "req-1",
            &IngestDecision {
                kind: KIND_LINK_MATCH.to_string(),
                library_id: library_id.clone(),
                payload_json: format!(r#"{{"mode":"link","item_id":"{item_id}"}}"#),
            },
        )
        .expect("record");
        let done = run_ingest_operation(&mut conn, &op.id, &transport).expect("run");
        assert_eq!(done.state, STATE_SUCCEEDED);
        let receipt = done.receipt_json.expect("receipt");
        assert!(receipt.contains("LIVE0001"), "receipt carries the key");
        assert!(
            receipt.contains("\"version\":3"),
            "receipt carries the version"
        );
    }

    #[test]
    fn link_rejects_unknown_tombstoned_and_foreign_works() {
        let (mut conn, library_id, item_id) = catalog_db();
        let transport = CatalogIngestTransport;
        // Tombstone the seeded work.
        conn.execute(
            "INSERT INTO zotero_item_tombstones (item_id, observed_at, reason) VALUES (?1, 1, 'deleted')",
            [&item_id],
        )
        .expect("tombstone");
        for (request, payload, code) in [
            (
                "req-unknown",
                format!(r#"{{"mode":"link","item_id":"nope"}}"#),
                "unknown_item",
            ),
            (
                "req-dead",
                format!(r#"{{"mode":"link","item_id":"{item_id}"}}"#),
                "tombstoned",
            ),
        ] {
            let op = record_ingest_decision(
                &conn,
                request,
                &IngestDecision {
                    kind: KIND_LINK_MATCH.to_string(),
                    library_id: library_id.clone(),
                    payload_json: payload,
                },
            )
            .expect("record");
            let failed = run_ingest_operation(&mut conn, &op.id, &transport).expect("run");
            assert_eq!(failed.state, STATE_FAILED);
            assert_eq!(failed.last_error_code.as_deref(), Some(code));
        }
    }

    #[test]
    fn create_stages_exactly_one_row_per_request() {
        let (mut conn, library_id, _) = catalog_db();
        let transport = CatalogIngestTransport;
        let decision = IngestDecision {
            kind: KIND_CREATE_PARENT.to_string(),
            library_id: library_id.clone(),
            payload_json: r#"{"mode":"create","title":"Obra nueva"}"#.to_string(),
        };
        let first = record_ingest_decision(&conn, "req-1", &decision).expect("record");
        let second = record_ingest_decision(&conn, "req-1", &decision).expect("re-record");
        assert_eq!(first.id, second.id);
        let done = run_ingest_operation(&mut conn, &first.id, &transport).expect("run");
        assert_eq!(done.state, STATE_SUCCEEDED);
        let receipt = done.receipt_json.expect("receipt");
        assert!(
            receipt.contains("\"version\":0"),
            "version 0 means staged locally"
        );
        let staged: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bibliographic_items WHERE library_id = ?1 AND item_version IS NULL",
                [&library_id],
                |row| row.get(0),
            )
            .expect("count staged");
        assert_eq!(staged, 1, "one request stages exactly one row");
    }

    #[test]
    fn create_links_on_title_collision_instead_of_duplicating() {
        let (mut conn, library_id, _) = catalog_db();
        let transport = CatalogIngestTransport;
        // Predict the derived key and pre-seed the same title under it.
        let key = derive_create_key("req-collide");
        use super::super::repository::{upsert_item, BibliographicItemInput};
        upsert_item(
            &mut conn,
            &library_id,
            BibliographicItemInput {
                item_key: key.clone(),
                item_version: Some(5),
                native_json_snapshot: format!(r#"{{"key":"{key}","version":5}}"#),
                csl_json_snapshot: format!(r#"{{"id":"{key}","title":"Obra gemela"}}"#),
                title: Some("Obra gemela".to_string()),
                ..Default::default()
            },
        )
        .expect("pre-seed collision");
        let op = record_ingest_decision(
            &conn,
            "req-collide",
            &IngestDecision {
                kind: KIND_CREATE_PARENT.to_string(),
                library_id: library_id.clone(),
                payload_json: r#"{"mode":"create","title":"Obra gemela"}"#.to_string(),
            },
        )
        .expect("record");
        let done = run_ingest_operation(&mut conn, &op.id, &transport).expect("run");
        assert_eq!(done.state, STATE_SUCCEEDED);
        let receipt = done.receipt_json.expect("receipt");
        assert!(receipt.contains(&key), "the existing record links");
        assert!(
            receipt.contains("\"version\":5"),
            "with its verified version"
        );
    }

    fn succeed_with_receipt(conn: &Connection, op_id: &str, receipt_json: &str) {
        conn.execute(
            "UPDATE bibliographic_ingest_operations SET state = 'running' WHERE id = ?1",
            [op_id],
        )
        .expect("start");
        complete_ingest_operation(conn, op_id, receipt_json).expect("complete");
    }

    #[test]
    fn gate_verifies_a_completed_link_receipt() {
        let (mut conn, library_id, _) = catalog_db();
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        succeed_with_receipt(
            &conn,
            &op.id,
            r#"{"item_key":"LIVE0001","version":3,"library":"0"}"#,
        );
        let work = verify_ingest_receipt(&conn, &op.id).expect("verify");
        assert_eq!(work.item_key, "LIVE0001");
        assert_eq!(work.version, 3);
        assert_eq!(work.library_row_id, library_id);
    }

    #[test]
    fn gate_rejects_incomplete_staged_stale_and_revoked() {
        let (mut conn, library_id, item_id) = catalog_db();
        // Incomplete: still queued.
        let queued = record_ingest_decision(&conn, "req-queued", &link_decision(&library_id))
            .expect("record");
        assert_eq!(
            verify_ingest_receipt(&conn, &queued.id)
                .expect_err("queued")
                .code,
            "not_completed"
        );
        // Staged: succeeded but version 0 (no Zotero write yet).
        let staged = record_ingest_decision(
            &conn,
            "req-staged",
            &IngestDecision {
                kind: KIND_CREATE_PARENT.to_string(),
                library_id: library_id.clone(),
                payload_json: r#"{"mode":"create","title":"Obra nueva"}"#.to_string(),
            },
        )
        .expect("record");
        succeed_with_receipt(
            &conn,
            &staged.id,
            r#"{"item_key":"WHATEVER","version":0,"library":"0"}"#,
        );
        assert_eq!(
            verify_ingest_receipt(&conn, &staged.id)
                .expect_err("staged")
                .code,
            "not_verified"
        );
        // Stale: receipt newer than the catalog.
        let stale = record_ingest_decision(&conn, "req-stale", &link_decision(&library_id))
            .expect("record");
        succeed_with_receipt(
            &conn,
            &stale.id,
            r#"{"item_key":"LIVE0001","version":9,"library":"0"}"#,
        );
        assert_eq!(
            verify_ingest_receipt(&conn, &stale.id)
                .expect_err("stale")
                .code,
            "stale_receipt"
        );
        // Revoked: tombstoned after completion.
        let dead =
            record_ingest_decision(&conn, "req-dead", &link_decision(&library_id)).expect("record");
        succeed_with_receipt(
            &conn,
            &dead.id,
            r#"{"item_key":"LIVE0001","version":3,"library":"0"}"#,
        );
        conn.execute(
            "INSERT INTO zotero_item_tombstones (item_id, observed_at, reason) VALUES (?1, 1, 'deleted')",
            [&item_id],
        )
        .expect("tombstone");
        assert_eq!(
            verify_ingest_receipt(&conn, &dead.id)
                .expect_err("revoked")
                .code,
            "tombstoned"
        );
    }

    fn seed_group_library(conn: &mut Connection) -> String {
        use super::super::repository::{
            upsert_connection, upsert_library, LibraryType, SourceOrigin, UpsertConnection,
            UpsertLibrary,
        };
        let source = upsert_connection(
            conn,
            UpsertConnection {
                id: "conn-group".to_string(),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://127.0.0.1:23119".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        upsert_library(
            conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::Group,
                library_id: "6680944".to_string(),
                name: "prueba".to_string(),
                last_modified_version: None,
            },
        )
        .expect("group library")
        .id
    }

    #[test]
    fn live_transport_refuses_foreign_libraries_without_http() {
        let mut conn = tray_db();
        let library_id = seed_library(&mut conn);
        let op =
            record_ingest_decision(&conn, "req-1", &link_decision(&library_id)).expect("record");
        let transport = ZoteroConnectorTransport::for_test_group("http://127.0.0.1:9", "6680944");
        let failure = transport.execute(&mut conn, &op).expect_err("must refuse");
        assert_eq!(failure.code, "wrong_library");
        assert!(failure.terminal);
    }

    /// Live round-trip against the isolated `prueba` group. Runs only
    /// with `ZSB_LIVE_ZOTERO=1` and a reachable local Zotero: it creates
    /// one clearly-labeled probe work (`ZSB live BORRAR …`) that the
    /// local API cannot delete afterwards — the group is the isolated
    /// test destination, so the probe stays there by design.
    #[test]
    fn live_transport_creates_in_the_test_group() {
        if std::env::var("ZSB_LIVE_ZOTERO").is_err() {
            eprintln!("skipping live Zotero write: ZSB_LIVE_ZOTERO is not set");
            return;
        }
        let mut conn = tray_db();
        let library_id = seed_group_library(&mut conn);
        let title = format!("ZSB live BORRAR {}", clock_ms());
        let op = record_ingest_decision(
            &conn,
            &format!("req-live-{}", clock_ms()),
            &IngestDecision {
                kind: KIND_CREATE_PARENT.to_string(),
                library_id: library_id.clone(),
                payload_json: format!(r#"{{"mode":"create","title":"{title}"}}"#),
            },
        )
        .expect("record");
        let transport =
            ZoteroConnectorTransport::for_test_group("http://127.0.0.1:23119", "6680944");
        let done = run_ingest_operation(&mut conn, &op.id, &transport).expect("run live");
        assert_eq!(done.state, STATE_SUCCEEDED);
        let receipt = done.receipt_json.expect("live receipt");
        let parsed: serde_json::Value = serde_json::from_str(&receipt).expect("receipt json");
        let key = parsed
            .get("item_key")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(key.len(), 8, "a native Zotero key");
        assert!(
            parsed.get("version").and_then(|v| v.as_u64()).unwrap_or(0) >= 1,
            "a Zotero-assigned version, never staged 0"
        );
        assert_eq!(
            parsed.get("library").and_then(|v| v.as_str()),
            Some("6680944")
        );
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
