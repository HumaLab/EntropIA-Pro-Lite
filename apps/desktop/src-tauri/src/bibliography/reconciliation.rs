//! Transaction-safe durable state for one Zotero library reconciliation run.
//!
//! This seam stores only synthetic native identity keys and bounded metadata.
//! It deliberately does not fetch Zotero, schedule work, resolve files, or
//! persist provider payloads. The connection revision is the stale-identity
//! fence; `zotero_libraries.last_modified_version` remains catalog metadata.

use super::repository::{BibliographyError, BibliographyResult};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MIGRATION_NAME: &str = "0040_bibliography_reconciliation";
const MAX_ERROR_CODE_LENGTH: usize = 128;
const MAX_ERROR_MESSAGE_LENGTH: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationState {
    Running,
    RetryWait,
    Interrupted,
    Blocked,
    Failed,
    Completed,
}

impl ReconciliationState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::Interrupted => "interrupted",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }

    fn from_db(value: String) -> BibliographyResult<Self> {
        match value.as_str() {
            "running" => Ok(Self::Running),
            "retry_wait" => Ok(Self::RetryWait),
            "interrupted" => Ok(Self::Interrupted),
            "blocked" => Ok(Self::Blocked),
            "failed" => Ok(Self::Failed),
            "completed" => Ok(Self::Completed),
            other => Err(storage_error(format!(
                "unknown reconciliation state {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationPhase {
    Versions,
    Catalog,
    Finalize,
}

impl ReconciliationPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Versions => "versions",
            Self::Catalog => "catalog",
            Self::Finalize => "finalize",
        }
    }

    fn from_db(value: String) -> BibliographyResult<Self> {
        match value.as_str() {
            "versions" => Ok(Self::Versions),
            "catalog" => Ok(Self::Catalog),
            "finalize" => Ok(Self::Finalize),
            other => Err(storage_error(format!(
                "unknown reconciliation phase {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationEntityKind {
    Item,
    Collection,
    Tag,
    Attachment,
}

impl ReconciliationEntityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Collection => "collection",
            Self::Tag => "tag",
            Self::Attachment => "attachment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationError {
    pub phase: ReconciliationPhase,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationRun {
    pub library_id: String,
    pub run_id: String,
    pub connection_revision: i64,
    pub state: ReconciliationState,
    pub phase: ReconciliationPhase,
    pub cursor_start: i64,
    pub cursor_limit: i64,
    pub remote_total: Option<i64>,
    pub target_version: Option<i64>,
    pub checkpoint_version: Option<i64>,
    pub retry_count: i64,
    pub attempt_count: i64,
    pub next_retry_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub latest_error: Option<ReconciliationError>,
    pub revision: i64,
    pub checkpointed_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginReconciliationInput {
    pub library_id: String,
    pub connection_revision: i64,
    #[serde(default)]
    pub cursor_start: i64,
    pub cursor_limit: i64,
    #[serde(default)]
    pub remote_total: Option<i64>,
    #[serde(default)]
    pub target_version: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationRunRef {
    pub library_id: String,
    pub run_id: String,
    pub connection_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationSeenInput {
    pub entity_kind: ReconciliationEntityKind,
    pub entity_key: String,
    #[serde(default)]
    pub parent_key: Option<String>,
    #[serde(default)]
    pub remote_version: Option<i64>,
    pub observed_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationPageInput {
    pub run: ReconciliationRunRef,
    pub phase: ReconciliationPhase,
    pub cursor_start: i64,
    pub next_cursor_start: i64,
    #[serde(default)]
    pub remote_total: Option<i64>,
    pub seen: Vec<ReconciliationSeenInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvanceReconciliationPhaseInput {
    pub run: ReconciliationRunRef,
    pub phase: ReconciliationPhase,
    pub next_phase: ReconciliationPhase,
    pub next_cursor_start: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationErrorInput {
    pub run: ReconciliationRunRef,
    pub expected_revision: i64,
    pub phase: ReconciliationPhase,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub next_retry_at: Option<i64>,
}

#[derive(Debug)]
struct RawRun {
    library_id: String,
    run_id: String,
    connection_revision: i64,
    state: String,
    phase: String,
    cursor_start: i64,
    cursor_limit: i64,
    remote_total: Option<i64>,
    target_version: Option<i64>,
    checkpoint_version: Option<i64>,
    retry_count: i64,
    attempt_count: i64,
    next_retry_at: Option<i64>,
    last_attempt_at: Option<i64>,
    latest_error_phase: Option<String>,
    latest_error_code: Option<String>,
    latest_error_message: Option<String>,
    latest_error_retryable: Option<i64>,
    latest_error_at: Option<i64>,
    revision: i64,
    checkpointed_at: Option<i64>,
    completed_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

fn error(code: &str, message: impl Into<String>) -> BibliographyError {
    BibliographyError {
        code: code.to_string(),
        message: message.into(),
    }
}

fn storage_error(message: impl Into<String>) -> BibliographyError {
    error("invalid_storage", message)
}

fn sql_error(context: &str, cause: rusqlite::Error) -> BibliographyError {
    error("sql_error", format!("{context}: {cause}"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn require_non_empty(value: &str, field: &str) -> BibliographyResult<()> {
    if value.trim().is_empty() {
        return Err(error("invalid_input", format!("{field} must not be empty")));
    }
    Ok(())
}

fn require_non_negative(value: i64, field: &str) -> BibliographyResult<()> {
    if value < 0 {
        return Err(error(
            "invalid_input",
            format!("{field} must be non-negative"),
        ));
    }
    Ok(())
}

fn validate_optional_non_negative(value: Option<i64>, field: &str) -> BibliographyResult<()> {
    if let Some(value) = value {
        require_non_negative(value, field)?;
    }
    Ok(())
}

fn validate_begin_input(input: &BeginReconciliationInput) -> BibliographyResult<()> {
    require_non_empty(&input.library_id, "library id")?;
    require_non_negative(input.connection_revision, "connection revision")?;
    require_non_negative(input.cursor_start, "cursor start")?;
    if input.cursor_limit <= 0 {
        return Err(error("invalid_input", "cursor limit must be positive"));
    }
    validate_optional_non_negative(input.remote_total, "remote total")?;
    validate_optional_non_negative(input.target_version, "target version")
}

fn validate_run_ref(run: &ReconciliationRunRef) -> BibliographyResult<()> {
    require_non_empty(&run.library_id, "library id")?;
    require_non_empty(&run.run_id, "run id")?;
    require_non_negative(run.connection_revision, "connection revision")
}

fn validate_seen_input(input: &ReconciliationSeenInput) -> BibliographyResult<()> {
    require_non_empty(&input.entity_key, "entity key")?;
    require_non_negative(input.observed_at, "observed at")?;
    validate_optional_non_negative(input.remote_version, "remote version")?;
    let parent_key = input.parent_key.as_deref().unwrap_or("");
    match input.entity_kind {
        ReconciliationEntityKind::Attachment => {
            require_non_empty(parent_key, "attachment parent key")?;
        }
        _ if !parent_key.is_empty() => {
            return Err(error(
                "invalid_input",
                "non-attachment parent key must be empty",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_page_input(input: &ReconciliationPageInput) -> BibliographyResult<()> {
    validate_run_ref(&input.run)?;
    require_non_negative(input.cursor_start, "cursor start")?;
    require_non_negative(input.next_cursor_start, "next cursor start")?;
    if input.next_cursor_start < input.cursor_start {
        return Err(error(
            "invalid_input",
            "next cursor start must not move backwards",
        ));
    }
    if input.next_cursor_start == input.cursor_start && !input.seen.is_empty() {
        return Err(error(
            "invalid_input",
            "a page with seen entities must advance the cursor",
        ));
    }
    validate_optional_non_negative(input.remote_total, "remote total")?;
    for seen in &input.seen {
        validate_seen_input(seen)?;
    }
    Ok(())
}

fn validate_error_input(input: &ReconciliationErrorInput) -> BibliographyResult<()> {
    validate_run_ref(&input.run)?;
    require_non_negative(input.expected_revision, "expected revision")?;
    require_non_empty(&input.code, "error code")?;
    require_non_empty(&input.message, "error message")?;
    if input.code.len() > MAX_ERROR_CODE_LENGTH {
        return Err(error("invalid_input", "error code is too long"));
    }
    if input.message.len() > MAX_ERROR_MESSAGE_LENGTH {
        return Err(error("invalid_input", "error message is too long"));
    }
    validate_optional_non_negative(input.next_retry_at, "next retry at")
}

fn connection_revision(conn: &Connection, library_id: &str) -> BibliographyResult<i64> {
    conn.query_row(
        "SELECT c.revision
           FROM zotero_libraries l
           JOIN zotero_connections c ON c.id = l.connection_id
          WHERE l.id = ?1",
        [library_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|cause| sql_error("Failed to read Zotero connection revision", cause))?
    .ok_or_else(|| error("not_found", "Zotero library does not exist"))
}

fn read_raw_run(conn: &Connection, library_id: &str) -> BibliographyResult<Option<RawRun>> {
    conn.query_row(
        "SELECT library_id, run_id, connection_revision, state, phase,
                cursor_start, cursor_limit, remote_total, target_version,
                checkpoint_version, retry_count, attempt_count, next_retry_at,
                last_attempt_at, latest_error_phase, latest_error_code,
                latest_error_message, latest_error_retryable, latest_error_at,
                revision, checkpointed_at, completed_at, created_at, updated_at
           FROM zotero_reconciliation_runs
          WHERE library_id = ?1",
        [library_id],
        |row| {
            Ok(RawRun {
                library_id: row.get(0)?,
                run_id: row.get(1)?,
                connection_revision: row.get(2)?,
                state: row.get(3)?,
                phase: row.get(4)?,
                cursor_start: row.get(5)?,
                cursor_limit: row.get(6)?,
                remote_total: row.get(7)?,
                target_version: row.get(8)?,
                checkpoint_version: row.get(9)?,
                retry_count: row.get(10)?,
                attempt_count: row.get(11)?,
                next_retry_at: row.get(12)?,
                last_attempt_at: row.get(13)?,
                latest_error_phase: row.get(14)?,
                latest_error_code: row.get(15)?,
                latest_error_message: row.get(16)?,
                latest_error_retryable: row.get(17)?,
                latest_error_at: row.get(18)?,
                revision: row.get(19)?,
                checkpointed_at: row.get(20)?,
                completed_at: row.get(21)?,
                created_at: row.get(22)?,
                updated_at: row.get(23)?,
            })
        },
    )
    .optional()
    .map_err(|cause| sql_error("Failed to read reconciliation run", cause))
}

fn build_run(raw: RawRun) -> BibliographyResult<ReconciliationRun> {
    let state = ReconciliationState::from_db(raw.state)?;
    let phase = ReconciliationPhase::from_db(raw.phase)?;
    let latest_error = match (
        raw.latest_error_phase,
        raw.latest_error_code,
        raw.latest_error_message,
        raw.latest_error_retryable,
        raw.latest_error_at,
    ) {
        (None, None, None, None, None) => None,
        (Some(phase), Some(code), Some(message), Some(retryable), Some(at)) => {
            let retryable = match retryable {
                0 => false,
                1 => true,
                other => return Err(storage_error(format!("invalid retry flag {other}"))),
            };
            Some(ReconciliationError {
                phase: ReconciliationPhase::from_db(phase)?,
                code,
                message,
                retryable,
                at,
            })
        }
        _ => return Err(storage_error("reconciliation error metadata is incomplete")),
    };

    Ok(ReconciliationRun {
        library_id: raw.library_id,
        run_id: raw.run_id,
        connection_revision: raw.connection_revision,
        state,
        phase,
        cursor_start: raw.cursor_start,
        cursor_limit: raw.cursor_limit,
        remote_total: raw.remote_total,
        target_version: raw.target_version,
        checkpoint_version: raw.checkpoint_version,
        retry_count: raw.retry_count,
        attempt_count: raw.attempt_count,
        next_retry_at: raw.next_retry_at,
        last_attempt_at: raw.last_attempt_at,
        latest_error,
        revision: raw.revision,
        checkpointed_at: raw.checkpointed_at,
        completed_at: raw.completed_at,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    })
}

fn read_run_optional(
    conn: &Connection,
    library_id: &str,
) -> BibliographyResult<Option<ReconciliationRun>> {
    read_raw_run(conn, library_id)?.map(build_run).transpose()
}

fn read_run_required(conn: &Connection, library_id: &str) -> BibliographyResult<ReconciliationRun> {
    read_run_optional(conn, library_id)?
        .ok_or_else(|| error("not_found", "reconciliation run does not exist"))
}

/// Reads the current reconciliation row for one internal library.
pub fn get_run(
    conn: &Connection,
    library_id: &str,
) -> BibliographyResult<Option<ReconciliationRun>> {
    require_non_empty(library_id, "library id")?;
    read_run_optional(conn, library_id)
}

fn validate_fence(
    conn: &Connection,
    run: &ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    validate_run_ref(run)?;
    let current_revision = connection_revision(conn, &run.library_id)?;
    if current_revision != run.connection_revision {
        return Err(error(
            "stale_connection",
            "connection revision no longer matches the reconciliation fence",
        ));
    }
    let stored = read_run_required(conn, &run.library_id)?;
    if stored.run_id != run.run_id {
        return Err(error(
            "stale_run",
            "reconciliation run is no longer current",
        ));
    }
    if stored.connection_revision != run.connection_revision {
        return Err(error(
            "stale_connection",
            "reconciliation run was created under another connection revision",
        ));
    }
    Ok(stored)
}

fn require_phase(run: &ReconciliationRun, expected: ReconciliationPhase) -> BibliographyResult<()> {
    if run.phase != expected {
        return Err(error(
            "stale_phase",
            "reconciliation phase no longer matches the caller fence",
        ));
    }
    Ok(())
}

fn require_state(
    run: &ReconciliationRun,
    allowed: &[ReconciliationState],
) -> BibliographyResult<()> {
    if allowed.iter().any(|state| *state == run.state) {
        return Ok(());
    }
    Err(error(
        "invalid_state",
        format!("reconciliation run is in state {:?}", run.state),
    ))
}

fn seen_key_exists(
    conn: &Connection,
    run: &ReconciliationRunRef,
    seen: &ReconciliationSeenInput,
) -> BibliographyResult<bool> {
    let parent_key = seen.parent_key.as_deref().unwrap_or("");
    conn.query_row(
        "SELECT 1 FROM zotero_reconciliation_seen
           WHERE library_id = ?1 AND run_id = ?2 AND entity_kind = ?3
             AND entity_key = ?4 AND parent_key = ?5",
        rusqlite::params![
            &run.library_id,
            &run.run_id,
            seen.entity_kind.as_str(),
            &seen.entity_key,
            parent_key,
        ],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(|cause| sql_error("Failed to read reconciliation seen key", cause))
}

/// Begins a fresh run for one library. Active and interrupted rows cannot be
/// replaced; completed, failed and blocked rows retain their last successful
/// checkpoint version while their old seen-set is cleared.
pub(crate) fn begin_run_in_transaction(
    conn: &Connection,
    input: BeginReconciliationInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_begin_input(&input)?;
    let current_revision = connection_revision(conn, &input.library_id)?;
    if current_revision != input.connection_revision {
        return Err(error(
            "stale_connection",
            "connection revision no longer matches the reconciliation fence",
        ));
    }

    let previous = read_run_optional(conn, &input.library_id)?;
    if let Some(previous) = &previous {
        match previous.state {
            ReconciliationState::Running | ReconciliationState::RetryWait => {
                return Err(error(
                    "active_run",
                    "an active reconciliation run already exists",
                ));
            }
            ReconciliationState::Interrupted => {
                return Err(error(
                    "interrupted_run",
                    "resume the interrupted reconciliation run explicitly",
                ));
            }
            ReconciliationState::Blocked
            | ReconciliationState::Failed
            | ReconciliationState::Completed => {}
        }
    }

    let run_id = Uuid::new_v4().to_string();
    let now = now_ms();
    conn.execute(
        "DELETE FROM zotero_reconciliation_seen WHERE library_id = ?1",
        [&input.library_id],
    )
    .map_err(|cause| {
        sql_error(
            "Failed to clear the previous reconciliation seen-set",
            cause,
        )
    })?;

    if let Some(previous) = previous {
        conn.execute(
            "UPDATE zotero_reconciliation_runs
                SET run_id = ?1,
                    connection_revision = ?2,
                    state = 'running',
                    phase = 'versions',
                    cursor_start = ?3,
                    cursor_limit = ?4,
                    remote_total = ?5,
                    target_version = ?6,
                    retry_count = 0,
                    attempt_count = 1,
                    next_retry_at = NULL,
                    last_attempt_at = ?7,
                    latest_error_phase = NULL,
                    latest_error_code = NULL,
                    latest_error_message = NULL,
                    latest_error_retryable = NULL,
                    latest_error_at = NULL,
                    revision = revision + 1,
                    checkpointed_at = NULL,
                    completed_at = NULL,
                    created_at = ?7,
                    updated_at = ?7
              WHERE library_id = ?8 AND revision = ?9",
            rusqlite::params![
                &run_id,
                input.connection_revision,
                input.cursor_start,
                input.cursor_limit,
                input.remote_total,
                input.target_version,
                now,
                &input.library_id,
                previous.revision,
            ],
        )
        .map_err(|cause| sql_error("Failed to begin a fresh reconciliation run", cause))?;
    } else {
        conn.execute(
            "INSERT INTO zotero_reconciliation_runs
                (library_id, run_id, connection_revision, state, phase,
                 cursor_start, cursor_limit, remote_total, target_version,
                 retry_count, attempt_count, last_attempt_at, revision,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, 'running', 'versions', ?4, ?5, ?6, ?7,
                     0, 1, ?8, 0, ?8, ?8)",
            rusqlite::params![
                &input.library_id,
                &run_id,
                input.connection_revision,
                input.cursor_start,
                input.cursor_limit,
                input.remote_total,
                input.target_version,
                now,
            ],
        )
        .map_err(|cause| sql_error("Failed to insert reconciliation run", cause))?;
    }

    read_run_required(conn, &input.library_id)
}

/// Public E1b wrapper retaining its one-operation transaction boundary.
pub fn begin_run(
    conn: &mut Connection,
    input: BeginReconciliationInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_begin_input(&input)?;
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation transaction", cause))?;
    let run = begin_run_in_transaction(&tx, input)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation run", cause))?;
    Ok(run)
}

/// Resumes durable work without changing its cursor or seen-set. The public
/// path accepts retry-wait/interrupted runs; scheduler convergence additionally
/// accepts blocked or explicitly retried failed runs.
pub(crate) fn resume_run_in_transaction(
    conn: &Connection,
    run: ReconciliationRunRef,
    allow_scheduler_retry: bool,
) -> BibliographyResult<ReconciliationRun> {
    let current = validate_fence(conn, &run)?;
    let allowed: &[ReconciliationState] = if allow_scheduler_retry {
        &[
            ReconciliationState::Interrupted,
            ReconciliationState::RetryWait,
            ReconciliationState::Blocked,
            ReconciliationState::Failed,
        ]
    } else {
        &[
            ReconciliationState::Interrupted,
            ReconciliationState::RetryWait,
        ]
    };
    require_state(&current, allowed)?;
    let now = now_ms();
    conn.execute(
        "UPDATE zotero_reconciliation_runs
            SET state = 'running', next_retry_at = NULL, attempt_count = attempt_count + 1,
                last_attempt_at = ?1, revision = revision + 1, updated_at = ?1
          WHERE library_id = ?2 AND run_id = ?3 AND revision = ?4",
        rusqlite::params![now, &run.library_id, &run.run_id, current.revision],
    )
    .map_err(|cause| sql_error("Failed to resume reconciliation run", cause))?;
    read_run_required(conn, &run.library_id)
}

pub fn resume_run(
    conn: &mut Connection,
    run: ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation resume transaction", cause))?;
    let resumed = resume_run_in_transaction(&tx, run, false)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation resume", cause))?;
    Ok(resumed)
}

/// Inserts one page's normalized seen keys and advances the cursor in one
/// transaction. Replaying an already advanced page is accepted only when all
/// of its keys are already present in the same run.
pub(crate) fn checkpoint_page_in_transaction(
    conn: &Connection,
    input: ReconciliationPageInput,
    target_version: Option<i64>,
) -> BibliographyResult<ReconciliationRun> {
    validate_page_input(&input)?;
    validate_optional_non_negative(target_version, "target version")?;
    let current = validate_fence(conn, &input.run)?;
    require_state(&current, &[ReconciliationState::Running])?;
    require_phase(&current, input.phase)?;
    if let (Some(known_total), Some(supplied_total)) = (current.remote_total, input.remote_total) {
        if supplied_total < known_total {
            return Err(error("stale_total", "remote total must not move backwards"));
        }
    }
    if let (Some(known_version), Some(supplied_version)) = (current.target_version, target_version)
    {
        if supplied_version != known_version {
            return Err(error(
                "stale_version",
                "library version changed during reconciliation",
            ));
        }
    }

    if current.cursor_start != input.cursor_start {
        let mut replay = current.cursor_start == input.next_cursor_start
            && input
                .remote_total
                .map(|remote_total| current.remote_total == Some(remote_total))
                .unwrap_or(true)
            && target_version
                .map(|version| current.target_version == Some(version))
                .unwrap_or(true);
        if replay {
            for seen in &input.seen {
                if !seen_key_exists(conn, &input.run, seen)? {
                    replay = false;
                    break;
                }
            }
        }
        if replay {
            return Ok(current);
        }
        return Err(error(
            "stale_cursor",
            "reconciliation cursor is no longer current",
        ));
    }

    for seen in &input.seen {
        let parent_key = seen.parent_key.as_deref().unwrap_or("");
        conn.execute(
            "INSERT OR IGNORE INTO zotero_reconciliation_seen
                (library_id, run_id, entity_kind, entity_key, parent_key, remote_version, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                &input.run.library_id,
                &input.run.run_id,
                seen.entity_kind.as_str(),
                &seen.entity_key,
                parent_key,
                seen.remote_version,
                seen.observed_at,
            ],
        )
        .map_err(|cause| sql_error("Failed to insert reconciliation seen key", cause))?;
    }

    let now = now_ms();
    let changed = conn
        .execute(
            "UPDATE zotero_reconciliation_runs
                SET cursor_start = ?1,
                    remote_total = COALESCE(?2, remote_total),
                    target_version = COALESCE(target_version, ?3),
                    checkpointed_at = ?4,
                    updated_at = ?4,
                    revision = revision + 1
              WHERE library_id = ?5 AND run_id = ?6 AND connection_revision = ?7
                AND state = 'running' AND phase = ?8 AND cursor_start = ?9
                AND revision = ?10",
            rusqlite::params![
                input.next_cursor_start,
                input.remote_total,
                target_version,
                now,
                &input.run.library_id,
                &input.run.run_id,
                input.run.connection_revision,
                input.phase.as_str(),
                input.cursor_start,
                current.revision,
            ],
        )
        .map_err(|cause| sql_error("Failed to advance reconciliation cursor", cause))?;
    if changed != 1 {
        return Err(error(
            "stale_revision",
            "reconciliation revision changed while checkpointing the page",
        ));
    }

    read_run_required(conn, &input.run.library_id)
}

pub fn checkpoint_page(
    conn: &mut Connection,
    input: ReconciliationPageInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_page_input(&input)?;
    let tx = conn.transaction().map_err(|cause| {
        sql_error(
            "Failed to begin reconciliation checkpoint transaction",
            cause,
        )
    })?;
    let checkpointed = checkpoint_page_in_transaction(&tx, input, None)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation page", cause))?;
    Ok(checkpointed)
}

/// Advances the explicit versions phase to the catalog phase without clearing
/// the run-scoped seen-set. Finalization owns the catalog-to-finalize transition.
pub(crate) fn advance_phase_in_transaction(
    conn: &Connection,
    input: AdvanceReconciliationPhaseInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_run_ref(&input.run)?;
    require_non_negative(input.next_cursor_start, "next cursor start")?;
    let current = validate_fence(conn, &input.run)?;
    require_state(&current, &[ReconciliationState::Running])?;
    require_phase(&current, input.phase)?;
    if input.phase != ReconciliationPhase::Versions
        || input.next_phase != ReconciliationPhase::Catalog
    {
        return Err(error(
            "invalid_phase",
            "only the versions-to-catalog phase transition is explicit",
        ));
    }

    let now = now_ms();
    conn.execute(
        "UPDATE zotero_reconciliation_runs
            SET phase = ?1, cursor_start = ?2, checkpointed_at = ?3,
                revision = revision + 1, updated_at = ?3
          WHERE library_id = ?4 AND run_id = ?5 AND phase = ?6
            AND state = 'running' AND revision = ?7",
        rusqlite::params![
            input.next_phase.as_str(),
            input.next_cursor_start,
            now,
            &input.run.library_id,
            &input.run.run_id,
            input.phase.as_str(),
            current.revision,
        ],
    )
    .map_err(|cause| sql_error("Failed to advance reconciliation phase", cause))?;
    read_run_required(conn, &input.run.library_id)
}

pub fn advance_phase(
    conn: &mut Connection,
    input: AdvanceReconciliationPhaseInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_run_ref(&input.run)?;
    require_non_negative(input.next_cursor_start, "next cursor start")?;
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation phase transaction", cause))?;
    let advanced = advance_phase_in_transaction(&tx, input)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation phase", cause))?;
    Ok(advanced)
}

/// Records bounded structured metadata for a retryable or terminal error. The
/// cursor and seen-set are intentionally untouched.
pub(crate) fn record_error_in_transaction(
    conn: &Connection,
    input: ReconciliationErrorInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_error_input(&input)?;
    let current = validate_fence(conn, &input.run)?;
    if current.revision != input.expected_revision {
        return Err(error(
            "stale_revision",
            "reconciliation revision no longer matches the error fence",
        ));
    }
    if input.retryable {
        require_state(&current, &[ReconciliationState::Running])?;
    } else {
        require_state(
            &current,
            &[ReconciliationState::Running, ReconciliationState::RetryWait],
        )?;
    }
    require_phase(&current, input.phase)?;

    let now = now_ms();
    let next_state = if input.retryable {
        ReconciliationState::RetryWait
    } else {
        ReconciliationState::Failed
    };
    let changed = conn
        .execute(
            "UPDATE zotero_reconciliation_runs
                SET state = ?1,
                    retry_count = retry_count + ?2,
                    next_retry_at = ?3,
                    latest_error_phase = ?4,
                    latest_error_code = ?5,
                    latest_error_message = ?6,
                    latest_error_retryable = ?7,
                    latest_error_at = ?8,
                    revision = revision + 1,
                    updated_at = ?8
              WHERE library_id = ?9 AND run_id = ?10 AND phase = ?4 AND revision = ?11",
            rusqlite::params![
                next_state.as_str(),
                if input.retryable { 1 } else { 0 },
                input.next_retry_at,
                input.phase.as_str(),
                &input.code,
                &input.message,
                if input.retryable { 1 } else { 0 },
                now,
                &input.run.library_id,
                &input.run.run_id,
                input.expected_revision,
            ],
        )
        .map_err(|cause| sql_error("Failed to record reconciliation error", cause))?;
    if changed != 1 {
        return Err(error(
            "stale_revision",
            "reconciliation revision changed while recording the error",
        ));
    }

    read_run_required(conn, &input.run.library_id)
}

pub fn record_error(
    conn: &mut Connection,
    input: ReconciliationErrorInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_error_input(&input)?;
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation error transaction", cause))?;
    let recorded = record_error_in_transaction(&tx, input)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation error", cause))?;
    Ok(recorded)
}

/// Parks a running or retry-wait run without touching its cursor or seen-set.
pub fn mark_interrupted(
    conn: &mut Connection,
    run: ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    transition_to_terminal_pause(conn, run, ReconciliationState::Interrupted)
}

/// Marks a run blocked by an explicit caller decision. No scheduler or retry is
/// invoked by this operation.
pub fn mark_blocked(
    conn: &mut Connection,
    run: ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    transition_to_terminal_pause(conn, run, ReconciliationState::Blocked)
}

pub(crate) fn transition_to_terminal_pause_in_transaction(
    conn: &Connection,
    run: ReconciliationRunRef,
    target: ReconciliationState,
) -> BibliographyResult<ReconciliationRun> {
    let current = validate_fence(conn, &run)?;
    if current.state == target {
        return Ok(current);
    }
    require_state(
        &current,
        &[
            ReconciliationState::Running,
            ReconciliationState::RetryWait,
            ReconciliationState::Interrupted,
        ],
    )?;
    let now = now_ms();
    conn.execute(
        "UPDATE zotero_reconciliation_runs
            SET state = ?1, next_retry_at = NULL, revision = revision + 1, updated_at = ?2
          WHERE library_id = ?3 AND run_id = ?4 AND revision = ?5",
        rusqlite::params![
            target.as_str(),
            now,
            &run.library_id,
            &run.run_id,
            current.revision,
        ],
    )
    .map_err(|cause| sql_error("Failed to change reconciliation state", cause))?;
    read_run_required(conn, &run.library_id)
}

fn transition_to_terminal_pause(
    conn: &mut Connection,
    run: ReconciliationRunRef,
    target: ReconciliationState,
) -> BibliographyResult<ReconciliationRun> {
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation state transaction", cause))?;
    let changed = transition_to_terminal_pause_in_transaction(&tx, run, target)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation state", cause))?;
    Ok(changed)
}

/// Completes a running run only through this explicit call. Known totals must
/// be reached; unknown totals are accepted because the caller explicitly
/// asserts completion. Only the target version can advance checkpoint_version,
/// and it can never move backwards.
pub(crate) fn finalize_run_in_transaction(
    conn: &Connection,
    run: ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    let current = validate_fence(conn, &run)?;
    if current.state == ReconciliationState::Completed
        && current.phase == ReconciliationPhase::Finalize
    {
        return Ok(current);
    }
    require_state(&current, &[ReconciliationState::Running])?;
    if current.phase == ReconciliationPhase::Finalize {
        return Err(error(
            "invalid_state",
            "reconciliation run is already in finalize phase",
        ));
    }
    if let Some(remote_total) = current.remote_total {
        if current.cursor_start < remote_total {
            return Err(error("incomplete_run", "remote total has not been reached"));
        }
    }

    let now = now_ms();
    let changed = conn
        .execute(
            "UPDATE zotero_reconciliation_runs
                SET state = 'completed', phase = 'finalize', next_retry_at = NULL,
                    checkpoint_version = CASE
                      WHEN target_version IS NULL THEN checkpoint_version
                      WHEN checkpoint_version IS NULL OR target_version > checkpoint_version
                        THEN target_version
                      ELSE checkpoint_version
                    END,
                    completed_at = ?1, revision = revision + 1, updated_at = ?1
              WHERE library_id = ?2 AND run_id = ?3 AND revision = ?4
                AND state = 'running' AND phase IN ('versions', 'catalog')",
            rusqlite::params![now, &run.library_id, &run.run_id, current.revision],
        )
        .map_err(|cause| sql_error("Failed to finalize reconciliation run", cause))?;
    if changed != 1 {
        return Err(error(
            "stale_revision",
            "reconciliation revision changed while finalizing the run",
        ));
    }
    read_run_required(conn, &run.library_id)
}

pub fn finalize_run(
    conn: &mut Connection,
    run: ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation finalize transaction", cause))?;
    let completed = finalize_run_in_transaction(&tx, run)?;
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation finalization", cause))?;
    Ok(completed)
}

/// Scheduler-specific convergence: reuse an active run, resume paused or
/// explicitly retried failed work, or begin after a completed run. The current
/// connection revision remains the identity fence: a run under another fence
/// is retired and restarted from zero, never resumed with foreign seen/cursor
/// state.
pub(crate) fn converge_run(
    conn: &mut Connection,
    input: BeginReconciliationInput,
) -> BibliographyResult<ReconciliationRun> {
    validate_begin_input(&input)?;
    let tx = conn
        .transaction()
        .map_err(|cause| sql_error("Failed to begin reconciliation convergence", cause))?;
    let current_revision = connection_revision(&tx, &input.library_id)?;
    if current_revision != input.connection_revision {
        return Err(error(
            "stale_connection",
            "connection revision no longer matches the reconciliation fence",
        ));
    }
    let existing = read_run_optional(&tx, &input.library_id)?;
    let run = match existing {
        None => begin_run_in_transaction(&tx, input)?,
        Some(run) if run.connection_revision != input.connection_revision => {
            // The connection identity fence changed. Never reuse this run's
            // cursor or seen-set under the new identity; retire it inside the
            // same write transaction, then begin from zero. Public E1b begin
            // semantics remain strict because only scheduler convergence owns
            // this stale-fence replacement.
            let changed = tx
                .execute(
                    "UPDATE zotero_reconciliation_runs
                        SET state = 'failed', next_retry_at = NULL,
                            revision = revision + 1, updated_at = ?1
                      WHERE library_id = ?2 AND run_id = ?3 AND revision = ?4",
                    rusqlite::params![now_ms(), &run.library_id, &run.run_id, run.revision],
                )
                .map_err(|cause| sql_error("Failed to retire stale reconciliation", cause))?;
            if changed != 1 {
                return Err(error(
                    "stale_revision",
                    "reconciliation changed while replacing a stale connection fence",
                ));
            }
            begin_run_in_transaction(&tx, input)?
        }
        Some(run) => match run.state {
            ReconciliationState::Running => run,
            ReconciliationState::Failed
                if run
                    .latest_error
                    .as_ref()
                    .is_some_and(|error| error.code == "zotero_snapshot_changed") =>
            {
                // This failure deliberately retires a mixed-version walk. Its
                // pages remain evidence, but retry must rebuild the seen-set
                // from zero under one trusted snapshot.
                begin_run_in_transaction(&tx, input)?
            }
            ReconciliationState::RetryWait
            | ReconciliationState::Interrupted
            | ReconciliationState::Blocked
            | ReconciliationState::Failed => resume_run_in_transaction(
                &tx,
                ReconciliationRunRef {
                    library_id: run.library_id,
                    run_id: run.run_id,
                    connection_revision: run.connection_revision,
                },
                true,
            )?,
            ReconciliationState::Completed => begin_run_in_transaction(&tx, input)?,
        },
    };
    tx.commit()
        .map_err(|cause| sql_error("Failed to commit reconciliation convergence", cause))?;
    Ok(run)
}

/// Trusted items-only finalization used by the scheduler publisher. It marks
/// every catalog item absent from this run's item seen-set with the existing
/// item-id tombstone identity, advances library version metadata, then closes
/// the run. The caller owns the surrounding success transaction.
pub(crate) fn finalize_items_run_in_transaction(
    conn: &Connection,
    run: ReconciliationRunRef,
) -> BibliographyResult<ReconciliationRun> {
    let current = validate_fence(conn, &run)?;
    if current.state == ReconciliationState::Completed
        && current.phase == ReconciliationPhase::Finalize
    {
        return Ok(current);
    }
    require_state(&current, &[ReconciliationState::Running])?;
    if let Some(remote_total) = current.remote_total {
        if current.cursor_start < remote_total {
            return Err(error("incomplete_run", "remote total has not been reached"));
        }
    }

    let now = now_ms();
    conn.execute(
        "INSERT INTO zotero_item_tombstones
           (item_id, observed_at, remote_version, reason)
         SELECT i.id, ?1, ?2, 'absent_from_completed_items_reconciliation'
           FROM bibliographic_items i
          WHERE i.library_id = ?3
            AND NOT EXISTS (
                SELECT 1 FROM zotero_reconciliation_seen s
                 WHERE s.library_id = ?3 AND s.run_id = ?4
                   AND s.entity_kind = 'item' AND s.entity_key = i.item_key
            )
         ON CONFLICT(item_id) DO UPDATE SET
            observed_at = excluded.observed_at,
            remote_version = excluded.remote_version,
            reason = excluded.reason",
        rusqlite::params![now, current.target_version, &run.library_id, &run.run_id,],
    )
    .map_err(|cause| sql_error("Failed to tombstone unseen bibliographic items", cause))?;

    if let Some(target_version) = current.target_version {
        conn.execute(
            "UPDATE zotero_libraries
                SET last_modified_version = ?1,
                    revision = revision + 1,
                    updated_at = ?2
              WHERE id = ?3
                AND (last_modified_version IS NULL OR last_modified_version < ?1)",
            rusqlite::params![target_version, now, &run.library_id],
        )
        .map_err(|cause| sql_error("Failed to advance Zotero library version", cause))?;
    }
    finalize_run_in_transaction(conn, run)
}

/// Recovery-side state convergence. Only active reconciliation rows owned by
/// bibliography tasks that this recovery transaction just parked are marked
/// interrupted; cursors, seen rows, completed runs and corpus tasks are not
/// touched.
pub(crate) fn interrupt_processing_runs_in_transaction(
    conn: &Connection,
) -> BibliographyResult<usize> {
    let now = now_ms();
    conn.execute(
        "UPDATE zotero_reconciliation_runs
            SET state = 'interrupted', next_retry_at = NULL,
                revision = revision + 1, updated_at = ?1
          WHERE state IN ('running', 'retry_wait')
            AND EXISTS (
                SELECT 1 FROM processing_tasks t
                 WHERE t.domain = 'bibliography'
                   AND t.subject_kind = 'library'
                   AND t.kind = 'bibliography_sync'
                   AND t.subject_id = zotero_reconciliation_runs.library_id
                   AND t.state = 'interrupted'
            )",
        [now],
    )
    .map_err(|cause| sql_error("Failed to interrupt bibliography reconciliation", cause))
}

// Descriptive aliases keep the seam discoverable to callers that prefer the
// full operation names while the short names remain convenient for Rust tests.
pub use advance_phase as advance_reconciliation_phase;
pub use begin_run as begin_reconciliation_run;
pub use checkpoint_page as checkpoint_reconciliation_page;
pub use finalize_run as finalize_reconciliation_run;
pub use get_run as get_reconciliation_run;
pub use mark_blocked as mark_reconciliation_blocked;
pub use mark_interrupted as mark_reconciliation_interrupted;
pub use record_error as record_reconciliation_error;
pub use resume_run as resume_interrupted_run;
