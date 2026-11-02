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

use super::approvals::{Approval, ApprovalAction, ApprovalRequest, ApprovalStore};
use super::read::MAX_LIMIT;
use crate::processing::commands::{
    self as batches, BatchSnapshotDto, BatchSummaryDto, ControlRequest, TaskQuery, TaskSummaryDto,
};
use crate::processing::repository;

/// Page size when the caller does not ask for one.
const DEFAULT_LIMIT: usize = 50;

/// The job changed between the caller's read and its request (S7's "conflicto
/// de revisión"): read it again and retry.
pub const ERROR_REVISION_CONFLICT: &str = "revision_conflict";

/// The job's current state does not admit the requested action.
pub const ERROR_INVALID_STATE: &str = "invalid_state";

/// The operation needs a provider the person has not configured (S7).
pub const ERROR_PROVIDER_NOT_CONFIGURED: &str = "provider_not_configured";

/// Longest request id an assistant may send.
const REQUEST_ID_MAX_CHARS: usize = 128;

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

/// `processing_prepare`'s payload: the draft and what to do next.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrepareReport {
    pub job_id: String,
    /// False when this request id already prepared this job.
    pub created: bool,
    /// Files in scope, each once.
    pub files: usize,
    pub operations: Vec<String>,
    pub state: String,
    /// The revision `processing_start` must name.
    pub revision: i64,
    /// What EntropIA knows about the cost at this point.
    pub cost_note: String,
    pub next_step: String,
}

/// `processing_control`'s payload: what happened to the job.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ControlReport {
    pub job_id: String,
    pub action: String,
    /// `done` when the action already applied (pause, cancel);
    /// `approval_pending` when it waits for the person (resume, retry).
    pub outcome: String,
    /// The job after the action, when it applied.
    pub job: Option<BatchSnapshotDto>,
    /// The request waiting for the person, when the action spends.
    pub approval: Option<Approval>,
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

/// Operations Lite runs through OpenRouter: the Procesamiento tab refuses them
/// without the key, and so does the endpoint.
fn needs_openrouter(operations: &[String]) -> bool {
    operations.iter().any(|op| {
        matches!(op.as_str(), "embeddings" | "ner" | "triples") || op.starts_with("schema:")
    })
}

/// Whether the OpenRouter key is configured. Reads the stored value (a keyring
/// reference or the legacy plain value) and never resolves the secret itself.
fn openrouter_configured(conn: &Connection) -> bool {
    crate::settings::get_raw_setting(conn, crate::settings::OPENROUTER_API_KEY)
        .is_some_and(|value| !value.trim().is_empty())
}

/// The operations a batch runs. The tab only ever sends these; an assistant's
/// typo is refused instead of silently dropped.
fn is_known_operation(op: &str) -> bool {
    matches!(op, "ocr" | "embeddings" | "ner" | "triples")
        || op.strip_prefix("schema:").is_some_and(|id| !id.is_empty())
}

fn check_request_id(request_id: &str) -> Result<(), JobsError> {
    let chars = request_id.chars().count();
    if request_id.trim().is_empty()
        || chars > REQUEST_ID_MAX_CHARS
        || request_id.chars().any(char::is_control)
    {
        return Err(JobsError::new(
            "invalid_argument",
            format!(
                "`requestId` tiene que tener entre 1 y {REQUEST_ID_MAX_CHARS} caracteres visibles."
            ),
        ));
    }
    Ok(())
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

    /// A connection that may write, to an archive whose batch queue exists.
    fn write_connection(&self) -> Result<Connection, JobsError> {
        batches::open_ready(&self.db_path).map_err(from_service)
    }

    /// Prepare a paused draft for a client. The request id is stored under
    /// the client (`agent:<client>:<request>`), so a retry returns the same
    /// job and two assistants can never collide on one id (D5).
    pub fn prepare(
        &self,
        client_id: &str,
        request_id: &str,
        collection_ids: &[String],
        item_ids: &[String],
        operations: &[String],
    ) -> Result<PrepareReport, JobsError> {
        check_request_id(request_id)?;
        if let Some(unknown) = operations.iter().find(|op| !is_known_operation(op)) {
            return Err(JobsError::new(
                "invalid_argument",
                format!(
                    "Operación desconocida \"{unknown}\": usá ocr, embeddings, ner, triples o schema:<id>."
                ),
            ));
        }
        if collection_ids.is_empty() && item_ids.is_empty() {
            return Err(JobsError::new(
                "invalid_argument",
                "Indicá al menos una colección (`collectionIds`) o un ítem (`itemIds`).",
            ));
        }
        let conn = self.write_connection()?;
        if !cfg!(feature = "local-ml")
            && needs_openrouter(operations)
            && !openrouter_configured(&conn)
        {
            return Err(JobsError::new(
                ERROR_PROVIDER_NOT_CONFIGURED,
                "Embeddings, entidades, tripletas y esquemas necesitan la clave de OpenRouter, que no está configurada. La persona puede cargarla en Configuración › Modelos; mientras tanto se puede preparar solo OCR.",
            ));
        }
        let prepared = batches::prepare_batch(
            &conn,
            &format!("agent:{client_id}:{request_id}"),
            collection_ids,
            item_ids,
            operations,
        )
        .map_err(from_service)?;
        let job = batches::batch_snapshot(&conn, &prepared.batch_id).map_err(from_service)?;
        Ok(PrepareReport {
            job_id: job.id,
            created: prepared.created,
            files: prepared.members,
            operations: job.operations,
            state: job.state,
            revision: job.revision,
            cost_note: "EntropIA no estima el costo al preparar: las páginas y unidades se cuentan al planificar. Cuando jobs_get muestre planningDone en true, tasksByKind dice cuántas unidades de cada operación hay. Lo que ya está procesado o compartido desde otro equipo no se vuelve a pagar.".to_string(),
            next_step: "Esperá a que jobs_get muestre state \"ready\"; después pedí empezar con processing_start, que la persona aprueba en EntropIA.".to_string(),
        })
    }

    /// Ask the owner to approve paid work on a job. The job must be at the
    /// revision the assistant names and in a state the action admits; the
    /// approval is bound to both (D3).
    pub fn request_approval(
        &self,
        approvals: &ApprovalStore,
        client_id: &str,
        client_name: &str,
        action: ApprovalAction,
        job_id: &str,
        revision: i64,
    ) -> Result<Approval, JobsError> {
        let conn = self.read_connection()?;
        let job = batches::batch_snapshot(&conn, job_id).map_err(from_service)?;
        if job.revision != revision {
            return Err(JobsError::new(
                ERROR_REVISION_CONFLICT,
                format!(
                    "El trabajo está en la revisión {}, no en la {revision}: volvé a leerlo con jobs_get y pedí de nuevo.",
                    job.revision
                ),
            ));
        }
        let terminal = matches!(
            job.state.as_str(),
            "completed" | "completed_with_errors" | "cancelled" | "cancelling"
        );
        let admitted = match action {
            ApprovalAction::Start => job.state == "ready",
            // A draft (preparing or ready) has never run: that is a start,
            // and it goes through processing_start's own check.
            ApprovalAction::Resume => {
                job.desired_state == "pause"
                    && !terminal
                    && !matches!(job.state.as_str(), "preparing" | "ready")
            }
            ApprovalAction::Retry { .. } => {
                !matches!(job.state.as_str(), "cancelled" | "cancelling")
            }
        };
        if !admitted {
            let what = match action {
                ApprovalAction::Start => {
                    "empezar (tiene que estar en \"ready\", con la planificación terminada)"
                }
                ApprovalAction::Resume => {
                    "reanudar (tiene que haber empezado y estar pausado; un borrador se empieza con processing_start)"
                }
                ApprovalAction::Retry { .. } => "reintentar (fue cancelado)",
            };
            return Err(JobsError::new(
                ERROR_INVALID_STATE,
                format!("El trabajo está en \"{}\" y no se puede {what}.", job.state),
            ));
        }
        Ok(approvals.request(ApprovalRequest {
            client_id: client_id.to_string(),
            client_name: client_name.to_string(),
            action,
            job_id: job.id,
            revision: job.revision,
            operations: job.operations,
            collections: job.collections.into_iter().map(|c| c.name).collect(),
            files: job.members_total,
        }))
    }

    /// Pause or cancel a job now. Both only withdraw demand: running units
    /// finish their current checkpoint and confirmed results are kept.
    pub fn control_now(
        &self,
        job_id: &str,
        action: &str,
        revision: Option<i64>,
    ) -> Result<BatchSnapshotDto, JobsError> {
        let conn = self.write_connection()?;
        let response = batches::control_batches(
            &conn,
            &ControlRequest {
                batch_id: Some(job_id.to_string()),
                action: action.to_string(),
                expected_revision: revision,
            },
        )
        .map_err(from_service)?;
        response.snapshot.ok_or_else(JobsError::unavailable)
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
