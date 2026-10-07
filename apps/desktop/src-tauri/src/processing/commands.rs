//! Tauri commands for the batch queue (plan-lote.md §11).
//!
//! Thin async shells: validate arguments, open a short archive connection,
//! run one repository call on the blocking pool, and return a snapshot. All
//! durability lives in `repository`/`recovery`; a lost IPC response never
//! means lost work — the client re-reads the snapshot with its `requestId`.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use super::repository::{self, BatchAction};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;

fn payload_hash(parts: &[&str]) -> String {
    let digest = Sha256::digest(parts.join("\u{0}").as_bytes());
    format!("{digest:x}")
}

fn open_ready(db_path: &std::path::Path) -> Result<Connection, String> {
    let conn = open_archive_connection(db_path)?;
    if !repository::is_schema_ready(&conn)? {
        return Err(format!(
            "{}: {} is not applied yet",
            repository::SCHEMA_NOT_READY,
            repository::MIGRATION_NAME
        ));
    }
    Ok(conn)
}

async fn blocking<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| format!("processing task failed: {e}"))?
}

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareResponse {
    pub batch_id: String,
    pub created: bool,
    pub members: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSnapshotDto {
    pub id: String,
    pub request_id: String,
    pub origin: String,
    pub state: String,
    pub desired_state: String,
    pub operations: Vec<String>,
    pub planning_cursor: i64,
    pub planning_done: bool,
    pub revision: i64,
    pub priority: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_error: Option<String>,
    pub members_total: i64,
    pub members_classified: i64,
    pub progress_done: i64,
    pub progress_total: Option<i64>,
    pub progress_unknown_tasks: i64,
    pub tasks_by_state: Vec<StateCountDto>,
    pub tasks_by_kind: Vec<StateCountDto>,
    pub collections: Vec<CollectionRefDto>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateCountDto {
    pub name: String,
    pub count: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRefDto {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummaryDto {
    pub id: String,
    pub state: String,
    pub desired_state: String,
    pub operations: Vec<String>,
    pub revision: i64,
    pub priority: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub active_units: i64,
    pub failed_units: i64,
    pub succeeded_units: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchCursorDto {
    pub created_at: i64,
    pub id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListBatchesResponse {
    pub batches: Vec<BatchSummaryDto>,
    pub next_cursor: Option<BatchCursorDto>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummaryDto {
    pub task_id: String,
    pub kind: String,
    pub asset_id: String,
    pub domain: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub state: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
    pub items_seen: Option<i64>,
    pub remote_total: Option<i64>,
    pub outcome: String,
    pub attempt_count: i64,
    pub retry_cycle: i64,
    pub next_retry_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub updated_at: i64,
    pub request_state: String,
    pub dependency_task_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub tasks: Vec<TaskSummaryDto>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptDto {
    pub attempt_number: i64,
    pub lease_epoch: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub outcome: String,
    pub retryable: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointDto {
    pub unit_key: String,
    pub checksum: String,
    pub created_at: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetailDto {
    pub task_id: String,
    pub kind: String,
    pub asset_id: String,
    pub domain: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub state: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
    pub items_seen: Option<i64>,
    pub remote_total: Option<i64>,
    pub outcome: String,
    pub attempt_count: i64,
    pub retry_cycle: i64,
    pub next_retry_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub checkpoints: Vec<CheckpointDto>,
    pub attempts: Vec<AttemptDto>,
    pub shared_with_batches: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlResponse {
    pub affected: usize,
    pub snapshot: Option<BatchSnapshotDto>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryResponse {
    pub reopened: usize,
    pub operation_id: String,
}

/// Durable answer for one manual per-library bibliography synchronization
/// request. `created=false` means the request attached to existing work;
/// `requeued=true` means it explicitly reopened interrupted, blocked, or
/// failed work.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BibliographySyncResponse {
    pub batch_id: String,
    pub task_id: String,
    pub created: bool,
    pub requeued: bool,
}

/// Why one kind of blocked derived work is parked: the stable code the
/// executor wrote beside the message it recorded. `None` while that kind
/// has nothing blocked.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyBlockedReason {
    pub code: Option<String>,
    pub message: Option<String>,
}

/// What one requested bibliography sync has actually done so far, read from
/// the task the scheduler owns. `state` is the task's own state vocabulary
/// (`pending`, `running`, `retry_wait`, `blocked`, `interrupted`, `succeeded`,
/// `failed`, `cancelled`); nothing here is inferred from the request.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BibliographySyncStatus {
    pub state: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub progress_done: i64,
    pub progress_total: Option<i64>,
    /// Works the finished sync walked, from its durable receipt.
    pub items_seen: Option<i64>,
    pub remote_total: Option<i64>,
    /// Derived work queued since the sync began: one profile per new or
    /// changed work, one extraction per new attachment. Zero on a finished
    /// sync means the library was already up to date.
    pub new_profiles: i64,
    pub new_extractions: i64,
    /// Live derived-work progress across the whole unsettled backlog window
    /// (the window `processing::repository::bibliography_sync_status`
    /// describes): distinct subjects of the kind, settled over queued — works
    /// for the profiles, attachments for the extractions. Each subject is
    /// classified by its latest task in the window (max rowid), so one work
    /// carrying several `bibliography_profile` tasks counts once and the
    /// screen can never show more fichas than the library has works. Keeps
    /// answering while the derived backlog drains after a success — and
    /// while the sync task itself is still `pending` or `running` — so the
    /// screen can follow the work instead of guessing.
    pub profiles_done: i64,
    pub profiles_total: i64,
    pub extractions_done: i64,
    pub extractions_total: i64,
    /// Live blocked derived work in the backlog window: subjects whose
    /// latest task is parked `blocked`, waiting for a change only the owner
    /// can make. Not settled and never counted as done — the screen says «en
    /// espera» about these.
    pub profiles_blocked: i64,
    pub extractions_blocked: i64,
    /// What the blocked work of each kind is parked on, read from each
    /// blocked subject's latest blocked task and named, per kind, by the
    /// first of those in window order (window order, stable across reads):
    /// the stable code the executor wrote (`configuration_required_embedding`
    /// when the embedding engine has no usable configuration,
    /// `configuration_required_ocr` when the OCR engine has none) beside its
    /// recorded message. Each kind names its own reason — profiles never
    /// borrow the extractions' — and `None` while that kind has nothing
    /// blocked.
    pub profiles_blocked_reason: Option<BibliographyBlockedReason>,
    pub extractions_blocked_reason: Option<BibliographyBlockedReason>,
    /// What the derived backlog still needs, in ms: the *actionable* remaining
    /// subjects of each kind times the average duration of that kind's finished
    /// attempts inside the backlog window. Blocked subjects are never timed —
    /// they wait on the owner, not on the clock — and when every remaining
    /// subject of a kind is blocked that kind has no estimate at all, so the
    /// answer is `None`. `None` as well while the window holds fewer than
    /// three finished attempts, or while any kind with actionable work left
    /// has fewer than three to average — an honest unknown, never a made-up
    /// number.
    pub eta_ms: Option<i64>,
}

/// Reads the durable status of one `bibliography_sync` task.
pub fn bibliography_sync_status(
    conn: &Connection,
    task_id: &str,
) -> Result<BibliographySyncStatus, String> {
    repository::bibliography_sync_status(conn, task_id)
}

/// Honest progress for the manual "synchronize library" button: the state of
/// the scheduler's task, so the UI never claims work that is not happening.
#[tauri::command]
pub async fn processing_bibliography_sync_status(
    task_id: String,
    db: State<'_, AppDbState>,
) -> Result<BibliographySyncStatus, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        bibliography_sync_status(&conn, &task_id)
    })
    .await
}

/// Honest progress for the restart-safe follower: the status of the newest
/// `bibliography_sync` task, or `null` when none was ever admitted. After an
/// app restart no sync was requested in this session, so this read is what
/// finds the derived backlog that may still be draining. Read-only.
#[tauri::command]
pub async fn processing_latest_bibliography_sync_status(
    db: State<'_, AppDbState>,
) -> Result<Option<BibliographySyncStatus>, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        repository::latest_bibliography_sync_status(&conn)
    })
    .await
}

fn snapshot_dto(snapshot: repository::BatchSnapshot) -> BatchSnapshotDto {
    BatchSnapshotDto {
        id: snapshot.id,
        request_id: snapshot.request_id,
        origin: snapshot.origin,
        state: snapshot.state,
        desired_state: snapshot.desired_state,
        operations: snapshot.operations,
        planning_cursor: snapshot.planning_cursor,
        planning_done: snapshot.planning_done,
        revision: snapshot.revision,
        priority: snapshot.priority,
        created_at: snapshot.created_at,
        updated_at: snapshot.updated_at,
        started_at: snapshot.started_at,
        finished_at: snapshot.finished_at,
        last_error: snapshot.last_error,
        members_total: snapshot.members_total,
        members_classified: snapshot.members_classified,
        progress_done: snapshot.progress_done,
        progress_total: snapshot.progress_total,
        progress_unknown_tasks: snapshot.progress_unknown_tasks,
        tasks_by_state: snapshot
            .tasks_by_state
            .into_iter()
            .map(|(name, count)| StateCountDto { name, count })
            .collect(),
        tasks_by_kind: snapshot
            .tasks_by_kind
            .into_iter()
            .map(|(name, count)| StateCountDto { name, count })
            .collect(),
        collections: snapshot
            .collections
            .into_iter()
            .map(|(id, name)| CollectionRefDto { id, name })
            .collect(),
    }
}

// ── Commands ────────────────────────────────────────────────────────────────

/// Transactional core for the manual bibliography command. A request id owns
/// one selected external namespace and one durable response, so a lost IPC
/// response replays without reapplying demand. New request ids pass through to
/// the repository's shared single-flight admission/requeue path.
pub fn apply_bibliography_sync_request(
    conn: &Connection,
    request_id: &str,
    library_type: &str,
    library_id: &str,
) -> Result<BibliographySyncResponse, String> {
    if request_id.trim().is_empty() {
        return Err("invalid_selection: a request id is required".to_string());
    }
    let hash = payload_hash(&[library_type, library_id]);
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| format!("Failed to begin bibliography sync request: {error}"))?;
    let applied = (|| {
        if let Some(previous) = repository::find_request(conn, request_id)? {
            if previous.payload_hash != hash {
                return Err(format!(
                    "invalid_selection: request {request_id} was already used with different parameters"
                ));
            }
            let response = previous.response_json.ok_or_else(|| {
                format!("invalid_storage: request {request_id} has no durable response")
            })?;
            return serde_json::from_str::<BibliographySyncResponse>(&response).map_err(|error| {
                format!("invalid_storage: request {request_id} has an invalid response: {error}")
            });
        }

        // Nothing else creates catalog rows: register the local connection
        // and this library first (same transaction, idempotent).
        repository::ensure_local_zotero_library(conn, library_type, library_id)?;
        let outcome = repository::admit_bibliography_sync_demand(conn, library_type, library_id)?;
        let response = BibliographySyncResponse {
            batch_id: outcome.batch_id,
            task_id: outcome.task_id,
            created: outcome.created,
            requeued: outcome.requeued,
        };
        let response_json = serde_json::to_string(&response)
            .map_err(|error| format!("Failed to encode bibliography sync response: {error}"))?;
        repository::record_request(
            conn,
            request_id,
            "bibliography_sync",
            Some(&response.batch_id),
            &hash,
            &response_json,
            repository::now_ms(),
        )?;
        Ok(response)
    })();
    match applied {
        Ok(response) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| format!("Failed to commit bibliography sync request: {error}"))?;
            Ok(response)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Manually admits or requeues synchronization for one unambiguous catalog
/// library. It schedules through the shared bibliography system batch and
/// returns immediately; the scheduler performs all Zotero page work.
#[tauri::command]
pub async fn processing_sync_bibliography_library(
    request_id: String,
    library_type: String,
    library_id: String,
    db: State<'_, AppDbState>,
) -> Result<BibliographySyncResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        apply_bibliography_sync_request(&conn, &request_id, &library_type, &library_id)
    })
    .await
}

/// Snapshots one batch scope durably and returns its id immediately.
/// Retried requests with the same `request_id` return the original batch;
/// the same id with a different payload is rejected, never forked.
#[tauri::command]
pub async fn processing_prepare(
    request_id: String,
    collection_ids: Vec<String>,
    operations: Vec<String>,
    db: State<'_, AppDbState>,
) -> Result<PrepareResponse, String> {
    if request_id.trim().is_empty() || collection_ids.is_empty() {
        return Err(
            "invalid_selection: a request id and at least one collection are required".to_string(),
        );
    }
    let mut ops: Vec<String> = operations
        .into_iter()
        .filter(|op| {
            matches!(op.as_str(), "ocr" | "embeddings" | "ner" | "triples")
                || op.strip_prefix("schema:").is_some_and(|id| !id.is_empty())
        })
        .collect();
    ops.sort();
    ops.dedup();
    if ops.is_empty() {
        return Err("invalid_selection: select OCR, embeddings, entities or triples".to_string());
    }
    let mut scope = collection_ids.clone();
    scope.sort();
    scope.dedup();
    let hash = payload_hash(&[&scope.join(","), &ops.join(",")]);
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
        let result = (|| {
        if let Some(previous) = repository::find_request(&conn, &request_id)? {
            if previous.payload_hash != hash {
                return Err(format!("invalid_selection: request {request_id} was already used with different parameters"));
            }
            let batch_id = previous.batch_id.ok_or_else(|| format!("invalid_selection: request {request_id} has no batch"))?;
            let members: usize = previous
                .response_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .and_then(|value| value.get("members")?.as_u64())
                .unwrap_or(0) as usize;
            return Ok(PrepareResponse { batch_id, created: false, members });
        }
        // Every referenced collection must exist: a typo must fail here, not
        // produce an empty batch that looks like "nothing to do".
        let placeholders = scope.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let found: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM collections WHERE id IN ({placeholders})"),
                rusqlite::params_from_iter(scope.iter()),
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to validate collections: {e}"))?;
        if found as usize != scope.len() {
            return Err("invalid_selection: one or more collections do not exist".to_string());
        }
        for schema_id in ops.iter().filter_map(|op| op.strip_prefix("schema:")) {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM extraction_schemas WHERE id = ?1)",
                    [schema_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("Failed to validate schema: {e}"))?;
            if !exists {
                return Err(format!("invalid_selection: schema {schema_id} does not exist"));
            }
        }
        let batch_id = format!("batch-{}", uuid::Uuid::new_v4());
        let ops_json = serde_json::to_string(&ops).map_err(|e| format!("Failed to encode operations: {e}"))?;
        let now = repository::now_ms();
        conn.execute(
            "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, created_at, updated_at)
             VALUES (?1, ?2, 'user', 'preparing', 'pause', ?3, ?4, ?4)",
            rusqlite::params![batch_id, request_id, ops_json, now],
        )
        .map_err(|e| format!("Failed to create batch: {e}"))?;
        conn.execute(
            &format!(
                "INSERT INTO processing_batch_collections (batch_id, collection_id_snapshot, name_snapshot)
                 SELECT ?1, id, name FROM collections WHERE id IN ({placeholders})"
            ),
            rusqlite::params_from_iter(std::iter::once(&batch_id).chain(scope.iter())),
        )
        .map_err(|e| format!("Failed to snapshot scope: {e}"))?;
        let members = repository::prepare_membership(&conn, &batch_id, &scope)?;
        if members == 0 {
            conn.execute(
                "UPDATE processing_batches SET planning_done = 1 WHERE id = ?1",
                [&batch_id],
            )
            .map_err(|e| format!("Failed to close empty planning: {e}"))?;
        }
        let response = serde_json::json!({ "batchId": batch_id, "members": members }).to_string();
        repository::record_request(&conn, &request_id, "prepare", Some(&batch_id), &hash, &response, now)?;
        Ok(PrepareResponse { batch_id, created: true, members })
        })();
        match result {
            Ok(response) => {
                conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(response)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    })
    .await
}

/// Confirms a prepared draft: planning runs to completion in the background
/// and execution starts. Answers from the durable row, not from the worker.
#[tauri::command]
pub async fn processing_start(
    batch_id: String,
    expected_revision: Option<i64>,
    db: State<'_, AppDbState>,
) -> Result<BatchSnapshotDto, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        let (state, revision): (String, i64) = conn
            .query_row(
                "SELECT state, revision FROM processing_batches WHERE id = ?1",
                [&batch_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| format!("invalid_selection: unknown batch {batch_id}"))?;
        if let Some(expected) = expected_revision {
            if expected != revision {
                return Err(format!("revision_conflict: batch {batch_id} is at revision {revision}, not {expected}"));
            }
        }
        if state != "ready" {
            return Err(format!("invalid_transition: batch {batch_id} is {state}, only a prepared draft can start"));
        }
        conn.execute(
            "UPDATE processing_batches SET desired_state = 'run', updated_at = strftime('%s', 'now') * 1000,
               revision = revision + 1 WHERE id = ?1",
            [&batch_id],
        )
        .map_err(|e| format!("Failed to start {batch_id}: {e}"))?;
        Ok(snapshot_dto(repository::read_batch_snapshot(&conn, &batch_id)?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequest {
    pub batch_id: Option<String>,
    pub action: String,
    pub expected_revision: Option<i64>,
}

fn validate_control_scope(request: &ControlRequest) -> Result<(), String> {
    if request.batch_id.is_none() && request.expected_revision.is_some() {
        return Err("invalid_selection: expected_revision requires a batch_id".to_string());
    }
    Ok(())
}

/// Pauses, resumes, or cancels one batch — or every live batch when
/// `batch_id` is absent (pause/resume only; cancelling everything at once is
/// rejected so a misclick cannot wipe the whole queue). Bulk control rejects
/// a scalar expected revision before opening the database; unfenced bulk calls
/// validate each target against its own current revision.
#[tauri::command]
pub async fn processing_control(
    request: ControlRequest,
    db: State<'_, AppDbState>,
) -> Result<ControlResponse, String> {
    let action = match request.action.as_str() {
        "pause" => BatchAction::Pause,
        "resume" => BatchAction::Resume,
        "cancel" => BatchAction::Cancel,
        other => return Err(format!("invalid_selection: unknown action {other}")),
    };
    validate_control_scope(&request)?;
    if request.batch_id.is_none() && action == BatchAction::Cancel {
        return Err("invalid_selection: cancelling every batch at once is not allowed".to_string());
    }
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        let targets: Vec<String> = match request.batch_id {
            Some(id) => vec![id],
            None => conn
                .prepare("SELECT id FROM processing_batches WHERE state NOT IN ('completed', 'completed_with_errors', 'cancelled')")
                .map_err(|e| format!("Failed to list live batches: {e}"))?
                .query_map([], |row| row.get(0))
                .map_err(|e| format!("Failed to list live batches: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to list live batches: {e}"))?,
        };
        let affected = targets.len();
        for id in &targets {
            repository::control_batch(&conn, id, action, request.expected_revision)?;
        }
        let snapshot = match targets.as_slice() {
            [single] => Some(snapshot_dto(repository::read_batch_snapshot(&conn, single)?)),
            _ => None,
        };
        Ok(ControlResponse { affected, snapshot })
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPriorityRequest {
    pub batch_id: String,
    pub priority: i64,
    pub expected_revision: Option<i64>,
}

/// Sets one batch scheduling priority (0 = background, 1 = high,
/// 2 = interactive) and answers with the fresh snapshot. Revision-fenced:
/// a stale UI fails closed instead of silently overriding a newer intent.
#[tauri::command]
pub async fn processing_set_priority(
    request: SetPriorityRequest,
    db: State<'_, AppDbState>,
) -> Result<BatchSnapshotDto, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        repository::set_batch_priority(
            &conn,
            &request.batch_id,
            request.priority,
            request.expected_revision,
        )?;
        Ok(snapshot_dto(repository::read_batch_snapshot(
            &conn,
            &request.batch_id,
        )?))
    })
    .await
}

/// Reopens failed units in a new retry cycle, pulling their failed
/// dependencies along. History is preserved; successes are never rerun.
/// Idempotent per `request_id`: double-clicks open exactly one cycle.
#[tauri::command]
pub async fn processing_retry(
    request_id: String,
    batch_id: String,
    task_id: Option<String>,
    failed_only: bool,
    db: State<'_, AppDbState>,
) -> Result<RetryResponse, String> {
    if request_id.trim().is_empty() {
        return Err("invalid_selection: a request id is required".to_string());
    }
    if task_id.is_none() && !failed_only {
        return Err("invalid_selection: retry needs a task or failed_only".to_string());
    }
    let hash = payload_hash(&[
        &batch_id,
        task_id.as_deref().unwrap_or(""),
        if failed_only { "failed" } else { "single" },
    ]);
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        if let Some(previous) = repository::find_request(&conn, &request_id)? {
            if previous.payload_hash != hash {
                return Err(format!("invalid_selection: request {request_id} was already used with different parameters"));
            }
            let reopened = previous
                .response_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .and_then(|value| value.get("reopened")?.as_u64())
                .unwrap_or(0) as usize;
            return Ok(RetryResponse { reopened, operation_id: request_id });
        }
        let reopened = repository::retry_failed(&conn, &batch_id, task_id.as_deref())?;
        let response = serde_json::json!({ "reopened": reopened }).to_string();
        repository::record_request(&conn, &request_id, "retry", Some(&batch_id), &hash, &response, repository::now_ms())?;
        Ok(RetryResponse { reopened, operation_id: request_id })
    })
    .await
}

#[tauri::command]
pub async fn processing_list_batches(
    states: Option<Vec<String>>,
    cursor_created_at: Option<i64>,
    cursor_id: Option<String>,
    limit: Option<usize>,
    db: State<'_, AppDbState>,
) -> Result<ListBatchesResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        let after = cursor_created_at.zip(cursor_id);
        let (batches, next) =
            repository::list_batches(&conn, states.as_deref(), after, limit.unwrap_or(50))?;
        Ok(ListBatchesResponse {
            batches: batches
                .into_iter()
                .map(|b| BatchSummaryDto {
                    id: b.id,
                    state: b.state,
                    desired_state: b.desired_state,
                    operations: b.operations,
                    revision: b.revision,
                    priority: b.priority,
                    created_at: b.created_at,
                    updated_at: b.updated_at,
                    active_units: b.active_units,
                    failed_units: b.failed_units,
                    succeeded_units: b.succeeded_units,
                })
                .collect(),
            next_cursor: next.map(|(created_at, id)| BatchCursorDto { created_at, id }),
        })
    })
    .await
}

#[tauri::command]
pub async fn processing_get_batch(
    batch_id: String,
    db: State<'_, AppDbState>,
) -> Result<BatchSnapshotDto, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        Ok(snapshot_dto(repository::read_batch_snapshot(
            &conn, &batch_id,
        )?))
    })
    .await
}

#[tauri::command]
pub async fn processing_list_tasks(
    batch_id: String,
    state: Option<String>,
    kind: Option<String>,
    after_task_id: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    db: State<'_, AppDbState>,
) -> Result<ListTasksResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        let (tasks, next) = repository::list_tasks(
            &conn,
            &batch_id,
            state.as_deref(),
            kind.as_deref(),
            after_task_id.as_deref(),
            limit.unwrap_or(50),
            offset.unwrap_or(0),
        )?;
        Ok(ListTasksResponse {
            tasks: tasks
                .into_iter()
                .map(|t| TaskSummaryDto {
                    task_id: t.task_id,
                    kind: t.kind,
                    asset_id: t.asset_id,
                    domain: t.domain,
                    subject_kind: t.subject_kind,
                    subject_id: t.subject_id,
                    state: t.state,
                    stage: t.stage,
                    progress_done: t.progress_done,
                    progress_total: t.progress_total,
                    items_seen: t.items_seen,
                    remote_total: t.remote_total,
                    outcome: t.outcome,
                    attempt_count: t.attempt_count,
                    retry_cycle: t.retry_cycle,
                    next_retry_at: t.next_retry_at,
                    error_code: t.error_code,
                    error_message: t.error_message,
                    updated_at: t.updated_at,
                    request_state: t.request_state,
                    dependency_task_id: t.dependency_task_id,
                })
                .collect(),
            next_cursor: next,
        })
    })
    .await
}

#[tauri::command]
pub async fn processing_get_task(
    batch_id: String,
    task_id: String,
    attempt_limit: Option<usize>,
    db: State<'_, AppDbState>,
) -> Result<TaskDetailDto, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        let detail =
            repository::read_task_detail(&conn, &batch_id, &task_id, attempt_limit.unwrap_or(20))?;
        Ok(TaskDetailDto {
            task_id: detail.task_id,
            kind: detail.kind,
            asset_id: detail.asset_id,
            domain: detail.domain,
            subject_kind: detail.subject_kind,
            subject_id: detail.subject_id,
            state: detail.state,
            stage: detail.stage,
            progress_done: detail.progress_done,
            progress_total: detail.progress_total,
            items_seen: detail.items_seen,
            remote_total: detail.remote_total,
            outcome: detail.outcome,
            attempt_count: detail.attempt_count,
            retry_cycle: detail.retry_cycle,
            next_retry_at: detail.next_retry_at,
            error_code: detail.error_code,
            error_message: detail.error_message,
            checkpoints: detail
                .checkpoints
                .into_iter()
                .map(|(unit_key, checksum, created_at)| CheckpointDto {
                    unit_key,
                    checksum,
                    created_at,
                })
                .collect(),
            attempts: detail
                .attempts
                .into_iter()
                .map(|a| AttemptDto {
                    attempt_number: a.attempt_number,
                    lease_epoch: a.lease_epoch,
                    started_at: a.started_at,
                    finished_at: a.finished_at,
                    outcome: a.outcome,
                    retryable: a.retryable,
                    error_code: a.error_code,
                    error_message: a.error_message,
                })
                .collect(),
            shared_with_batches: detail.shared_with_batches,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_control_rejects_one_revision_for_divergent_targets() {
        let error = validate_control_scope(&ControlRequest {
            batch_id: None,
            action: "pause".to_string(),
            expected_revision: Some(7),
        })
        .expect_err("one revision cannot fence a bulk target set");
        assert_eq!(
            error,
            "invalid_selection: expected_revision requires a batch_id"
        );

        assert!(validate_control_scope(&ControlRequest {
            batch_id: None,
            action: "pause".to_string(),
            expected_revision: None,
        })
        .is_ok());
        assert!(validate_control_scope(&ControlRequest {
            batch_id: Some("batch-1".to_string()),
            action: "pause".to_string(),
            expected_revision: Some(7),
        })
        .is_ok());
    }
}
