//! The processing tools' adapter: the batch services of
//! [`crate::processing::commands`] under the agent tool contract.
//!
//! No rule about batches lives here. The services own validation, idempotency
//! and revision fencing for the Procesamiento tab and the endpoint alike (S6);
//! this module only translates: page limits refused rather than clamped, opaque
//! cursors a caller can hand back but never build, and service errors turned
//! into the stable codes and Spanish sentences every tool answers with (S7).
//! Reads use a `query_only` connection, like the archive tools in
//! [`super::read`].

use std::path::PathBuf;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rusqlite::Connection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::read::MAX_LIMIT;
use crate::processing::commands::{
    self as batches, BatchSnapshotDto, BatchSummaryDto, TaskQuery, TaskSummaryDto,
};
use crate::processing::repository;

/// Page size when the caller does not ask for one.
const DEFAULT_LIMIT: usize = 50;

/// The job changed between the caller's read and its request (S7's "conflicto
/// de revisión"): read it again and retry.
pub const ERROR_REVISION_CONFLICT: &str = "revision_conflict";

/// The job's current state does not admit the requested action.
pub const ERROR_INVALID_STATE: &str = "invalid_state";

/// A refusal under the tool contract: a stable code and one sentence for the
/// person reading the assistant's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobsError {
    pub code: &'static str,
    pub message: String,
}

impl JobsError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self::new(
            "unavailable",
            "No se pudo leer el procesamiento en este momento. Reintentá en unos segundos; si el problema sigue, revisá el archivo desde EntropIA.",
        )
    }
}

/// Translate an error from the batch services. Their messages start with a
/// machine prefix (`invalid_selection:`, `revision_conflict:`, …) followed by a
/// detail that names only ids and states, so the detail can travel to the
/// assistant. Anything without a known prefix may name the archive path: it
/// stays in the app's log and the caller gets `unavailable`.
pub(crate) fn from_service(error: String) -> JobsError {
    // "unknown batch state <x>" is a bad filter, not a missing job.
    let unknown_job = error
        .strip_prefix("invalid_selection: unknown batch ")
        .filter(|rest| !rest.starts_with("state "));
    if let Some(id) = unknown_job {
        return JobsError::new(
            "not_found",
            format!("No hay un trabajo con id {id} en este archivo."),
        );
    }
    if let Some(detail) = error.strip_prefix("invalid_selection: ") {
        return JobsError::new(
            "invalid_argument",
            format!("La petición no es válida: {detail}."),
        );
    }
    if let Some(detail) = error.strip_prefix("revision_conflict: ") {
        return JobsError::new(
            ERROR_REVISION_CONFLICT,
            format!("El trabajo cambió desde la última lectura ({detail}). Volvé a leerlo con jobs_get y reintentá."),
        );
    }
    if let Some(detail) = error.strip_prefix("invalid_transition: ") {
        return JobsError::new(
            ERROR_INVALID_STATE,
            format!("El trabajo no admite esa acción en su estado actual ({detail})."),
        );
    }
    log::warn!(target: "automation", "a processing request failed: {error}");
    JobsError::unavailable()
}

/// `jobs_list`'s payload.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobsPage {
    pub records: Vec<BatchSummaryDto>,
    /// Hand it back as `cursor` for the next page; `null` on the last one.
    pub next_cursor: Option<String>,
}

/// `jobs_get`'s payload.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobDetail {
    pub job: BatchSnapshotDto,
    pub tasks: Vec<TaskSummaryDto>,
    /// Hand it back as `tasksCursor` for the next page; `null` on the last one.
    pub next_tasks_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobsCursor {
    created_at: i64,
    id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TasksCursor {
    after: String,
}

fn encode<T: Serialize>(cursor: &T) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).unwrap_or_default())
}

fn decode<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, JobsError> {
    URL_SAFE_NO_PAD
        .decode(raw)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            JobsError::new(
                "invalid_argument",
                "El cursor no es válido: pasá tal cual el que devolvió la página anterior.",
            )
        })
}

fn resolve_limit(limit: Option<u32>) -> Result<usize, JobsError> {
    let limit = limit.map_or(DEFAULT_LIMIT, |limit| limit as usize);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(JobsError::new(
            "invalid_argument",
            format!("El límite tiene que estar entre 1 y {MAX_LIMIT}: llegó {limit}."),
        ));
    }
    Ok(limit)
}

/// The processing state of one archive, as the tools see it.
#[derive(Debug, Clone)]
pub struct JobsArchive {
    db_path: PathBuf,
}

impl JobsArchive {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }

    /// A `query_only` connection to an archive whose batch queue exists.
    fn read_connection(&self) -> Result<Connection, JobsError> {
        let conn = crate::db::open::open_archive_connection(&self.db_path).map_err(from_service)?;
        conn.execute_batch("PRAGMA query_only = ON;")
            .map_err(|error| from_service(error.to_string()))?;
        if !repository::is_schema_ready(&conn).map_err(from_service)? {
            return Err(JobsError::unavailable());
        }
        Ok(conn)
    }

    /// One page of the person's batches, newest first. System containers
    /// (bibliography sync, repairs) are not listed, exactly as in the tab.
    pub fn list(
        &self,
        states: Option<Vec<String>>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<JobsPage, JobsError> {
        let limit = resolve_limit(limit)?;
        let after = cursor
            .map(decode::<JobsCursor>)
            .transpose()?
            .map(|cursor| (cursor.created_at, cursor.id));
        let conn = self.read_connection()?;
        let page = batches::list_batches_page(&conn, states.as_deref(), after, limit)
            .map_err(from_service)?;
        Ok(JobsPage {
            records: page.batches,
            next_cursor: page.next_cursor.map(|next| {
                encode(&JobsCursor {
                    created_at: next.created_at,
                    id: next.id,
                })
            }),
        })
    }

    /// One batch and one page of its tasks.
    pub fn get(
        &self,
        job_id: &str,
        task_state: Option<&str>,
        task_kind: Option<&str>,
        tasks_cursor: Option<&str>,
        tasks_limit: Option<u32>,
    ) -> Result<JobDetail, JobsError> {
        let limit = resolve_limit(tasks_limit)?;
        let after = tasks_cursor
            .map(decode::<TasksCursor>)
            .transpose()?
            .map(|cursor| cursor.after);
        let conn = self.read_connection()?;
        let job = batches::batch_snapshot(&conn, job_id).map_err(from_service)?;
        let tasks = batches::list_batch_tasks(
            &conn,
            job_id,
            &TaskQuery {
                state: task_state,
                kind: task_kind,
                after_task_id: after.as_deref(),
                limit,
                offset: 0,
            },
        )
        .map_err(from_service)?;
        Ok(JobDetail {
            job,
            tasks: tasks.tasks,
            next_tasks_cursor: tasks
                .next_cursor
                .map(|after| encode(&TasksCursor { after })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_errors_become_the_codes_the_tool_contract_publishes() {
        let cases = [
            ("invalid_selection: unknown batch batch-1", "not_found"),
            (
                "invalid_selection: unknown batch state x",
                "invalid_argument",
            ),
            (
                "invalid_selection: unknown task state x",
                "invalid_argument",
            ),
            (
                "revision_conflict: batch b is at revision 3, not 2",
                ERROR_REVISION_CONFLICT,
            ),
            (
                "invalid_transition: batch b is already completed",
                ERROR_INVALID_STATE,
            ),
            (
                "Failed to read batch b: disk I/O error at /home/x/entropia.sqlite",
                "unavailable",
            ),
        ];
        for (service, code) in cases {
            assert_eq!(from_service(service.to_string()).code, code, "{service}");
        }
    }

    #[test]
    fn an_unprefixed_error_never_reaches_the_assistant() {
        let error = from_service("Failed to open /home/agustin/secret/entropia.sqlite".into());
        assert!(!error.message.contains("/home"), "{}", error.message);
    }
}
