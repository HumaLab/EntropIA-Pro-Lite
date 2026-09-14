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
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_error: Option<String>,
    pub members_total: i64,
    pub members_classified: i64,
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
    pub state: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
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
    pub state: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
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
        created_at: snapshot.created_at,
        updated_at: snapshot.updated_at,
        started_at: snapshot.started_at,
        finished_at: snapshot.finished_at,
        last_error: snapshot.last_error,
        members_total: snapshot.members_total,
        members_classified: snapshot.members_classified,
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
        .filter(|op| op == "ocr" || op == "embeddings")
        .collect();
    ops.sort();
    ops.dedup();
    if ops.is_empty() {
        return Err("invalid_selection: select OCR, embeddings, or both".to_string());
    }
    let mut scope = collection_ids.clone();
    scope.sort();
    scope.dedup();
    let hash = payload_hash(&[&scope.join(","), &ops.join(",")]);
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_ready(&db_path)?;
        if let Some((batch_id, stored_hash, response)) = conn
            .query_row(
                "SELECT batch_id, payload_hash, response_json FROM processing_requests WHERE request_id = ?1",
                [&request_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(format!("Failed to check request {request_id}: {other}")),
            })?
        {
            if stored_hash != hash {
                return Err(format!("invalid_selection: request {request_id} was already used with different parameters"));
            }
            let batch_id = batch_id.ok_or_else(|| format!("invalid_selection: request {request_id} has no batch"))?;
            let members: usize = response
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
        conn.execute(
            "INSERT INTO processing_requests (request_id, action, batch_id, payload_hash, state, response_json, created_at)
             VALUES (?1, 'prepare', ?2, ?3, 'applied', ?4, ?5)",
            rusqlite::params![request_id, batch_id, hash, response, now],
        )
        .map_err(|e| format!("Failed to record request: {e}"))?;
        Ok(PrepareResponse { batch_id, created: true, members })
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
        if state != "preparing" && state != "ready" {
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

/// Pauses, resumes, or cancels one batch — or every live batch when
/// `batch_id` is absent (pause/resume only; cancelling everything at once is
/// rejected so a misclick cannot wipe the whole queue).
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
        if let Some((stored_hash, response)) = conn
            .query_row(
                "SELECT payload_hash, response_json FROM processing_requests WHERE request_id = ?1",
                [&request_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(format!("Failed to check request {request_id}: {other}")),
            })?
        {
            if stored_hash != hash {
                return Err(format!("invalid_selection: request {request_id} was already used with different parameters"));
            }
            let reopened = response
                .as_deref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .and_then(|value| value.get("reopened")?.as_u64())
                .unwrap_or(0) as usize;
            return Ok(RetryResponse { reopened, operation_id: request_id });
        }
        let reopened = repository::retry_failed(&conn, &batch_id, task_id.as_deref())?;
        let response = serde_json::json!({ "reopened": reopened }).to_string();
        conn.execute(
            "INSERT INTO processing_requests (request_id, action, batch_id, payload_hash, state, response_json, created_at)
             VALUES (?1, 'retry', ?2, ?3, 'applied', ?4, ?5)",
            rusqlite::params![request_id, batch_id, hash, response, repository::now_ms()],
        )
        .map_err(|e| format!("Failed to record request: {e}"))?;
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
        )?;
        Ok(ListTasksResponse {
            tasks: tasks
                .into_iter()
                .map(|t| TaskSummaryDto {
                    task_id: t.task_id,
                    kind: t.kind,
                    asset_id: t.asset_id,
                    state: t.state,
                    stage: t.stage,
                    progress_done: t.progress_done,
                    progress_total: t.progress_total,
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
            state: detail.state,
            stage: detail.stage,
            progress_done: detail.progress_done,
            progress_total: detail.progress_total,
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
