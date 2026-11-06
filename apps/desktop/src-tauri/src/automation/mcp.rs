//! What the endpoint offers over MCP: one small tool per business intent (S6).
//!
//! One [`AgentServer`] is built per request (the transport runs stateless), so
//! nothing outlives a request and no identity is cached between them: every tool
//! reads the caller from the `Principal` the endpoint's guard put into the HTTP
//! request (D6).
//!
//! Three rules hold for every tool in this file:
//!
//! - **Scope first.** A client without [`Scope::Read`] gets `forbidden_scope`
//!   before any archive is touched, so a refusal can never leak a row (S8, D10).
//! - **Strict arguments.** Every argument struct denies unknown fields, and the
//!   schema published in `tools/list` says so. A bad argument is a tool error
//!   with a stable code, never a silent fallback.
//! - **Audited.** Every call — allowed or refused — appends one line to the
//!   local trail (S9): who, which tool, how it ended, which ids, and for a search
//!   only its length. Never a query, never a token.
//!
//! There is no generic SQL, shell, `eval` or "invoke any command" tool, and this
//! file must never grow one (S6, D10).

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::automation::access::{AgentAccess, Principal, Scope};
use crate::automation::approvals::{Approval, ApprovalAction, ApprovalStore};
use crate::automation::audit::{AuditLine, AuditLog};
use crate::automation::corpus_tools::{
    CorpusArchive, CorpusEvents, CreatedCollection, ImportRequest, ImportSources, IndexedItem,
    TextReplacement, UpdatedItem,
};
use crate::automation::jobs::{
    ControlReport, JobDetail, JobsArchive, JobsError, JobsPage, PrepareReport,
};
use crate::automation::read::{
    ArchiveCounts, ArchiveReader, CollectionsPage, DocumentText, ItemDetail, ItemsPage,
    PassageResults, ReadError, SearchResults,
};
use crate::automation::server::{app_version, profile_name, variant_name, PROTOCOL};
use crate::corpus::{CollectionDto, CollectionPatch};

/// Name the endpoint answers `initialize` with.
pub const SERVER_NAME: &str = "entropia";

/// The caller is paired but lacks the scope this tool needs (D10).
pub const ERROR_FORBIDDEN_SCOPE: &str = "forbidden_scope";

/// The id the caller named does not exist in this archive (D10).
pub const ERROR_NOT_FOUND: &str = "not_found";

/// The caller's arguments cannot be used as given (D10).
pub const ERROR_INVALID_ARGUMENT: &str = "invalid_argument";

/// The archive could not be read right now (S7's "capacidad no disponible").
pub const ERROR_UNAVAILABLE: &str = "unavailable";

const TOOL_STATUS: &str = "entropia_status";
const TOOL_COLLECTIONS_LIST: &str = "collections_list";
const TOOL_ITEMS_LIST: &str = "items_list";
const TOOL_ITEMS_GET: &str = "items_get";
const TOOL_SEARCH_ITEMS: &str = "search_items";
const TOOL_DOCUMENTS_GET_TEXT: &str = "documents_get_text";
const TOOL_SEARCH_PASSAGES: &str = "search_passages";
const TOOL_JOBS_LIST: &str = "jobs_list";
const TOOL_JOBS_GET: &str = "jobs_get";
const TOOL_PROCESSING_PREPARE: &str = "processing_prepare";
const TOOL_PROCESSING_START: &str = "processing_start";
const TOOL_PROCESSING_CONTROL: &str = "processing_control";
const TOOL_APPROVALS_STATUS: &str = "approvals_status";
const TOOL_COLLECTIONS_CREATE: &str = "collections_create";
const TOOL_COLLECTIONS_UPDATE: &str = "collections_update";
const TOOL_ITEMS_UPDATE: &str = "items_update";
const TOOL_DOCUMENTS_TEXT_UPDATE: &str = "documents_text_update";
const TOOL_DOCUMENTS_INDEX: &str = "documents_index";
const TOOL_IMPORT_SOURCES_LIST: &str = "import_sources_list";
const TOOL_DOCUMENTS_IMPORT: &str = "documents_import";
const TOOL_DOCUMENTS_DELETE: &str = "documents_delete";
const TOOL_COLLECTIONS_DELETE: &str = "collections_delete";

const REFUSE_LOCAL_WRITE: &str = "Este asistente no tiene permiso para preparar ni controlar procesamiento: la persona puede dárselo en Configuración › Asistentes («Preparar procesamiento y editar el corpus»).";
const REFUSE_REMOTE_SPEND: &str = "Este asistente no tiene permiso para pedir procesamiento pago: la persona puede dárselo en Configuración › Asistentes («Pedir procesamiento pago»).";

/// `entropia_status`'s payload: this instance, this archive, this caller.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    /// Protocol the app and the connector agree on.
    pub protocol: String,
    pub app_version: String,
    /// `pro` or `lite`, from the compiled feature set.
    pub variant: String,
    /// Dev profile name, `null` in a release build.
    pub profile: Option<String>,
    /// The paired client making the call.
    pub client: CallerInfo,
    /// How much the archive holds, so an assistant can tell "empty" from
    /// "not found".
    pub archive: ArchiveCounts,
    /// Every tool this endpoint serves, in `tools/list` order.
    pub tools: Vec<String>,
}

/// The authenticated caller, without its token.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CallerInfo {
    pub id: String,
    pub name: String,
    pub scopes: Vec<Scope>,
}

/// A tool refusal the assistant can branch on and show verbatim.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolErrorBody {
    /// Stable code: `forbidden_scope`, `not_found`, `invalid_argument`,
    /// `unavailable`.
    pub error: String,
    /// One sentence for the person reading the assistant's output.
    pub message: String,
}

/// `entropia_status` takes no arguments, and refuses any it is given.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusParams {}

/// `collections_list` arguments (D10).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionsListParams {
    /// The `nextCursor` the previous page returned. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size, 1 to 200; the default is 50.
    pub limit: Option<u32>,
}

/// `items_list` arguments (D10).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemsListParams {
    /// Only the items of this collection.
    pub collection_id: Option<String>,
    /// The `nextCursor` the previous page returned. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size, 1 to 200; the default is 50.
    pub limit: Option<u32>,
}

/// `items_get` arguments (D10).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemsGetParams {
    /// The item's stable id, as `items_list` or `search_items` returned it.
    pub item_id: String,
}

/// `search_items` arguments (D10).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchItemsParams {
    /// What to look for, in natural language.
    pub query: String,
    /// Only the items of this collection.
    pub collection_id: Option<String>,
    /// How many hits to return, 1 to 200; the default is 50.
    pub limit: Option<u32>,
}

/// `search_passages` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchPassagesParams {
    /// What to look for, in natural language.
    pub query: String,
    /// Only passages of this collection's items.
    pub collection_id: Option<String>,
    /// Only passages of this item.
    pub item_id: Option<String>,
    /// How many passages to return, 1 to 200; the default is 50.
    pub limit: Option<u32>,
}

/// `documents_get_text` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentsGetTextParams {
    /// The item whose text to read.
    pub item_id: String,
    /// Only this one of the item's files.
    pub asset_id: Option<String>,
    /// The `nextCursor` the previous call returned.
    pub cursor: Option<String>,
    /// Characters per call across pages, 1 to 20000; the default is 4000.
    pub max_chars: Option<u32>,
}

/// `jobs_list` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobsListParams {
    /// Only jobs in these states: `preparing`, `ready`, `running`, `pausing`,
    /// `paused`, `cancelling`, `cancelled`, `interrupted`, `completed`,
    /// `completed_with_errors`.
    pub states: Option<Vec<String>>,
    /// The `nextCursor` the previous page returned. Omit for the first page.
    pub cursor: Option<String>,
    /// Page size, 1 to 200; the default is 50.
    pub limit: Option<u32>,
}

/// `jobs_get` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobsGetParams {
    /// The job's id, as `jobs_list` or `processing_prepare` returned it.
    pub job_id: String,
    /// Only tasks in this state: `pending`, `blocked`, `running`, `retry_wait`,
    /// `interrupted`, `succeeded`, `failed`, `skipped`, `cancelled`.
    pub task_state: Option<String>,
    /// Only tasks of this kind: `ocr`, `embedding`, `ner`, `triples`,
    /// `schema_extract`, `ocr_correction`, `summary`.
    pub task_kind: Option<String>,
    /// The `nextTasksCursor` the previous call returned.
    pub tasks_cursor: Option<String>,
    /// Tasks per page, 1 to 200; the default is 50.
    pub tasks_limit: Option<u32>,
}

/// `processing_prepare` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessingPrepareParams {
    /// Your own id for this request, 1 to 128 characters. Sending it again
    /// with the same arguments returns the same job instead of a second one.
    pub request_id: String,
    /// Collections whose files enter the job.
    pub collection_ids: Option<Vec<String>>,
    /// Items whose files enter the job.
    pub item_ids: Option<Vec<String>>,
    /// `ocr`, `embeddings`, `ner`, `triples`, `ocr_correction`, `summary`, or
    /// `schema:<id>`.
    pub operations: Vec<String>,
}

/// `collections_create` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionsCreateParams {
    /// Your own id for this request, 1 to 128 characters; resending it with the
    /// same arguments returns the same collection instead of a second one.
    pub request_id: String,
    pub name: String,
    pub description: Option<String>,
}

/// `collections_update` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionsUpdateParams {
    pub collection_id: String,
    /// The collection's `updatedAt` as you read it.
    pub revision: i64,
    pub name: Option<String>,
    /// `null` clears the description; omit it to leave it as it is.
    #[serde(default, deserialize_with = "double_option")]
    #[schemars(with = "Option<String>")]
    pub description: Option<Option<String>>,
}

/// `items_update` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemsUpdateParams {
    pub item_id: String,
    /// The item's `updatedAt` as you read it.
    pub revision: i64,
    pub title: Option<String>,
    /// Every metadata field, as text values: it replaces the item's fields.
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
}

/// `documents_text_update` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentsTextUpdateParams {
    pub item_id: String,
    /// The page (file) whose text to replace.
    pub asset_id: String,
    /// The page's `textHash` as `documents_get_text` returned it.
    pub text_hash: String,
    /// The whole new text of the page.
    pub text: String,
}

/// `import_sources_list` takes no arguments, and refuses any it is given.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportSourcesListParams {}

/// `documents_import` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentsImportParams {
    /// The collection the files go into.
    pub collection_id: String,
    /// One of the folders `import_sources_list` returned, as it returned it.
    pub folder: String,
    /// File names inside that folder (no subfolders), 1 to 200.
    pub files: Vec<String>,
}

/// `documents_index` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentsIndexParams {
    pub item_id: String,
}

/// `documents_delete` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentsDeleteParams {
    /// The documents to delete, 1 to 200 ids, as `items_list` returned them.
    pub item_ids: Vec<String>,
}

/// `collections_delete` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionsDeleteParams {
    /// The collection to delete with everything it holds.
    pub collection_id: String,
}

/// Tells an absent field from an explicit `null`.
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// `processing_start` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessingStartParams {
    /// The job to start, in state `ready`.
    pub job_id: String,
    /// The `revision` jobs_get returned: the person approves exactly that state.
    pub revision: i64,
}

/// `processing_control` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessingControlParams {
    pub job_id: String,
    /// `pause` or `cancel` (apply now), `resume` or `retry` (wait for the
    /// person's approval).
    pub action: String,
    /// The job's `revision` from jobs_get. Required for `resume` and `retry`;
    /// optional for `pause` and `cancel`, where it refuses a stale request.
    pub revision: Option<i64>,
    /// `retry` only: the one task to reopen.
    pub task_id: Option<String>,
    /// `retry` only: reopen every failed task.
    pub failed_only: Option<bool>,
}

/// `approvals_status` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalsStatusParams {
    /// The `id` processing_start or processing_control returned.
    pub approval_id: String,
}

/// Arguments that deserialize strictly but fail as a tool error.
///
/// rmcp turns a failed `Parameters` extraction into a plain-text tool result, and
/// D10 asks every tool error to be structured (`{error, message}`) with a stable
/// code. Deserializing lazily here — while publishing the inner type's schema,
/// which is the strict one — keeps both: unknown fields and wrong types are
/// refused by `deny_unknown_fields` and still answered as `invalid_argument`.
pub struct Strict<T>(pub Result<T, String>);

impl<T: JsonSchema> JsonSchema for Strict<T> {
    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::json_schema(generator)
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Strict<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self(
            serde_json::from_value(value).map_err(|e| e.to_string()),
        ))
    }
}

#[derive(Clone)]
pub struct AgentServer {
    tool_router: ToolRouter<Self>,
    reader: ArchiveReader,
    jobs: JobsArchive,
    /// Where paid requests wait for the owner (D3); shared with the app.
    approvals: Arc<ApprovalStore>,
    corpus: CorpusArchive,
    /// Who may connect and which folders imports may read.
    access: Arc<AgentAccess>,
    audit: AuditLog,
}

impl AgentServer {
    /// `db_path` is the archive the tools read; `cache_dir` receives the audit
    /// trail under `logs/`; `approvals` is where paid requests wait.
    pub fn new(
        db_path: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        approvals: Arc<ApprovalStore>,
        corpus_events: Arc<CorpusEvents>,
        access: Arc<AgentAccess>,
    ) -> Self {
        let db_path = db_path.into();
        Self {
            tool_router: Self::tool_router(),
            approvals,
            access,
            corpus: CorpusArchive::new(db_path.clone(), corpus_events),
            jobs: JobsArchive::new(db_path.clone()),
            reader: ArchiveReader::new(db_path),
            audit: AuditLog::new(cache_dir.into().join("logs")),
        }
    }

    /// The caller the endpoint authenticated for this request.
    ///
    /// The transport copies [`http::request::Parts`] into the tool context and
    /// the guard put the [`Principal`] in those parts' extensions. A missing
    /// principal means the request bypassed the guard, which is a server bug:
    /// no tool may fall back to "some client".
    fn caller(context: &RequestContext<RoleServer>) -> Result<Principal, CallToolResult> {
        context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>())
            .cloned()
            .ok_or_else(|| {
                tool_error(
                    "internal_error",
                    "La petición llegó sin un cliente autenticado; esto es un fallo de EntropIA, no tuyo."
                        .to_string(),
                )
            })
    }

    /// The caller, provided it may read. Records the refusal before returning it.
    fn guard(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
    ) -> Result<Principal, CallToolResult> {
        self.guard_scope(
            context,
            tool,
            Scope::Read,
            "Este asistente no tiene permiso de lectura en EntropIA: pedile a la persona que lo emparejó que revise sus permisos.",
        )
    }

    /// The caller, provided it holds `scope`; otherwise the refusal, audited,
    /// before any archive is touched.
    fn guard_scope(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
        scope: Scope,
        refusal: &str,
    ) -> Result<Principal, CallToolResult> {
        let caller = Self::caller(context)?;
        if caller.has(scope) {
            return Ok(caller);
        }
        self.audit_error(&caller, tool, ERROR_FORBIDDEN_SCOPE, None);
        Err(tool_error(ERROR_FORBIDDEN_SCOPE, refusal.to_string()))
    }

    /// Arguments, or the audit and refusal that turn down what cannot be parsed.
    fn arguments<T>(
        &self,
        caller: &Principal,
        tool: &str,
        args: Strict<T>,
    ) -> Result<T, CallToolResult>
    where
        T: DeserializeOwned,
    {
        match args.0 {
            Ok(args) => Ok(args),
            Err(reason) => {
                self.audit_error(caller, tool, ERROR_INVALID_ARGUMENT, None);
                Err(tool_error(
                    ERROR_INVALID_ARGUMENT,
                    format!("Los argumentos de la herramienta no son válidos: {reason}"),
                ))
            }
        }
    }

    /// Audit a normal answer: which objects it touched, and for a search how
    /// long the query was.
    fn audit_ok(
        &self,
        caller: &Principal,
        tool: &str,
        object_ids: Vec<String>,
        query_chars: Option<usize>,
    ) {
        self.audit
            .append(&AuditLine::ok(caller, tool, object_ids, query_chars));
    }

    /// Audit a refused answer: the stable code, never the arguments.
    fn audit_error(&self, caller: &Principal, tool: &str, code: &str, query_chars: Option<usize>) {
        self.audit
            .append(&AuditLine::failed(caller, tool, code, query_chars));
    }

    /// Record and answer a failed read.
    fn read_failure(
        &self,
        caller: &Principal,
        tool: &str,
        error: ReadError,
        query_chars: Option<usize>,
    ) -> CallToolResult {
        self.audit_error(caller, tool, error.code(), query_chars);
        tool_error(error.code(), error.message().to_string())
    }

    /// Refuse a caller that lacks the scope an action needs, audited.
    fn refuse_scope(&self, caller: &Principal, tool: &str, message: &str) -> CallToolResult {
        self.audit_error(caller, tool, ERROR_FORBIDDEN_SCOPE, None);
        tool_error(ERROR_FORBIDDEN_SCOPE, message.to_string())
    }

    /// Refuse arguments that parsed but cannot be used, audited.
    fn refuse_argument(&self, caller: &Principal, tool: &str, message: &str) -> CallToolResult {
        self.audit_error(caller, tool, ERROR_INVALID_ARGUMENT, None);
        tool_error(ERROR_INVALID_ARGUMENT, message.to_string())
    }

    /// Ask the owner to approve one paid action, off the async runtime.
    async fn ask_owner(
        &self,
        caller: &Principal,
        tool: &str,
        action: ApprovalAction,
        job_id: String,
        revision: i64,
    ) -> Result<Approval, CallToolResult> {
        let jobs = self.jobs.clone();
        let approvals = self.approvals.clone();
        let client_id = caller.client_id.clone();
        let client_name = caller.name.clone();
        match Self::on_jobs(move || {
            jobs.request_approval(
                &approvals,
                &client_id,
                &client_name,
                action,
                &job_id,
                revision,
            )
        })
        .await
        {
            Ok(approval) => {
                self.audit_ok(
                    caller,
                    tool,
                    vec![approval.job_id.clone(), approval.id.clone()],
                    None,
                );
                Ok(approval)
            }
            Err(error) => Err(self.jobs_failure(caller, tool, error)),
        }
    }

    /// Record and answer a failed processing request.
    fn jobs_failure(&self, caller: &Principal, tool: &str, error: JobsError) -> CallToolResult {
        self.audit_error(caller, tool, error.code, None);
        tool_error(error.code, error.message)
    }

    /// Run one blocking processing call off the async runtime.
    async fn on_jobs<T, F>(work: F) -> Result<T, JobsError>
    where
        F: FnOnce() -> Result<T, JobsError> + Send + 'static,
        T: Send + 'static,
    {
        match tokio::task::spawn_blocking(work).await {
            Ok(result) => result,
            Err(error) => {
                log::warn!(target: "automation", "the processing task did not finish: {error}");
                Err(JobsError::unavailable())
            }
        }
    }

    /// Run one blocking archive read off the async runtime.
    async fn on_archive<T, F>(work: F) -> Result<T, ReadError>
    where
        F: FnOnce() -> Result<T, ReadError> + Send + 'static,
        T: Send + 'static,
    {
        match tokio::task::spawn_blocking(work).await {
            Ok(result) => result,
            Err(error) => {
                // A panic message can name the archive path; it stays in the log
                // for the same reason `read::unavailable` keeps its detail there.
                log::warn!(target: "automation", "the archive read task did not finish: {error}");
                Err(ReadError::Unavailable(
                    "No se pudo leer el archivo en este momento. Reintentá en unos segundos; si el problema sigue, revisá el archivo desde EntropIA."
                        .to_string(),
                ))
            }
        }
    }
}

/// Build the structured tool error D10 requires: a stable code, one Spanish
/// sentence, and `isError: true`.
fn tool_error(code: &str, message: String) -> CallToolResult {
    let body = ToolErrorBody {
        error: code.to_string(),
        message,
    };
    let value = serde_json::to_value(body).unwrap_or_else(|_| serde_json::json!({ "error": code }));
    CallToolResult::structured_error(value)
}

#[tool_router(router = tool_router)]
impl AgentServer {
    #[tool(
        name = "entropia_status",
        description = "Estado de esta instancia de EntropIA: versión, variante, perfil, cliente conectado, cuántas colecciones e ítems tiene el archivo y qué herramientas ofrece.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn entropia_status(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<StatusParams>>,
    ) -> Result<Json<StatusReport>, CallToolResult> {
        let caller = self.guard(&context, TOOL_STATUS)?;
        // No arguments defined, so any field at all is refused (D10).
        self.arguments(&caller, TOOL_STATUS, args)?;
        let reader = self.reader.clone();
        let archive = match Self::on_archive(move || reader.counts()).await {
            Ok(counts) => counts,
            Err(error) => return Err(self.read_failure(&caller, TOOL_STATUS, error, None)),
        };
        let tools = self
            .tool_router
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        self.audit_ok(&caller, TOOL_STATUS, Vec::new(), None);
        Ok(Json(StatusReport {
            protocol: PROTOCOL.to_string(),
            app_version: app_version().to_string(),
            variant: variant_name().to_string(),
            profile: profile_name(),
            client: CallerInfo {
                id: caller.client_id,
                name: caller.name,
                scopes: caller.scopes,
            },
            archive,
            tools,
        }))
    }

    #[tool(
        name = "collections_list",
        description = "Colecciones del archivo, ordenadas por nombre, con cuántos ítems tiene cada una. Pagina con `cursor` (el `nextCursor` anterior) y `limit` (1 a 200, por defecto 50); `nextCursor` es null en la última página.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn collections_list(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<CollectionsListParams>>,
    ) -> Result<Json<CollectionsPage>, CallToolResult> {
        let caller = self.guard(&context, TOOL_COLLECTIONS_LIST)?;
        let args = self.arguments(&caller, TOOL_COLLECTIONS_LIST, args)?;
        let reader = self.reader.clone();
        let page =
            match Self::on_archive(move || reader.collections(args.cursor.as_deref(), args.limit))
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    return Err(self.read_failure(&caller, TOOL_COLLECTIONS_LIST, error, None))
                }
            };
        let object_ids = page
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect();
        self.audit_ok(&caller, TOOL_COLLECTIONS_LIST, object_ids, None);
        Ok(Json(page))
    }

    #[tool(
        name = "items_list",
        description = "Ítems del archivo, ordenados por título, con su colección y su revisión (`updatedAt`). Filtra por `collectionId`. Pagina con `cursor` (el `nextCursor` anterior) y `limit` (1 a 200, por defecto 50); `nextCursor` es null en la última página.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn items_list(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ItemsListParams>>,
    ) -> Result<Json<ItemsPage>, CallToolResult> {
        let caller = self.guard(&context, TOOL_ITEMS_LIST)?;
        let args = self.arguments(&caller, TOOL_ITEMS_LIST, args)?;
        let reader = self.reader.clone();
        let page = match Self::on_archive(move || {
            reader.items(
                args.collection_id.as_deref(),
                args.cursor.as_deref(),
                args.limit,
            )
        })
        .await
        {
            Ok(page) => page,
            Err(error) => return Err(self.read_failure(&caller, TOOL_ITEMS_LIST, error, None)),
        };
        let object_ids = page
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect();
        self.audit_ok(&caller, TOOL_ITEMS_LIST, object_ids, None);
        Ok(Json(page))
    }

    #[tool(
        name = "items_get",
        description = "Un ítem por su id: título, colección, metadatos, revisión y la lista de archivos adjuntos. No devuelve el texto: para leerlo por páginas está `documents_get_text`.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn items_get(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ItemsGetParams>>,
    ) -> Result<Json<ItemDetail>, CallToolResult> {
        let caller = self.guard(&context, TOOL_ITEMS_GET)?;
        let args = self.arguments(&caller, TOOL_ITEMS_GET, args)?;
        let reader = self.reader.clone();
        let detail = match Self::on_archive(move || reader.item(&args.item_id)).await {
            Ok(detail) => detail,
            Err(error) => return Err(self.read_failure(&caller, TOOL_ITEMS_GET, error, None)),
        };
        self.audit_ok(&caller, TOOL_ITEMS_GET, vec![detail.id.clone()], None);
        Ok(Json(detail))
    }

    #[tool(
        name = "search_items",
        description = "Busca en el texto indexado del archivo y devuelve los ítems que coinciden, del más al menos relevante. Filtra por `collectionId` y limita con `limit` (1 a 200, por defecto 50); `matches` dice cuántas coincidencias hay en total.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search_items(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<SearchItemsParams>>,
    ) -> Result<Json<SearchResults>, CallToolResult> {
        let caller = self.guard(&context, TOOL_SEARCH_ITEMS)?;
        let args = self.arguments(&caller, TOOL_SEARCH_ITEMS, args)?;
        // Only the length is ever audited, so compute it before the query moves.
        let query_chars = Some(args.query.chars().count());
        let reader = self.reader.clone();
        let results = match Self::on_archive(move || {
            reader.search(&args.query, args.collection_id.as_deref(), args.limit)
        })
        .await
        {
            Ok(results) => results,
            Err(error) => {
                return Err(self.read_failure(&caller, TOOL_SEARCH_ITEMS, error, query_chars))
            }
        };
        let object_ids = results
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect();
        self.audit_ok(&caller, TOOL_SEARCH_ITEMS, object_ids, query_chars);
        Ok(Json(results))
    }

    #[tool(
        name = "search_passages",
        description = "Busca pasajes del texto de los documentos (fragmentos, no ítems enteros), del más al menos relevante, cada uno con su ítem, archivo, página, posiciones `startChar`/`endChar` y `textHash` para citarlo; `current` es false si el texto de esa página cambió después de indexarlo. Solo tienen pasajes los documentos procesados con embeddings: `coverage` dice cuántos ítems del alcance los tienen. Filtra por `collectionId` o `itemId` y limita con `limit` (1 a 200, por defecto 50).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search_passages(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<SearchPassagesParams>>,
    ) -> Result<Json<PassageResults>, CallToolResult> {
        let caller = self.guard(&context, TOOL_SEARCH_PASSAGES)?;
        let args = self.arguments(&caller, TOOL_SEARCH_PASSAGES, args)?;
        let query_chars = Some(args.query.chars().count());
        let reader = self.reader.clone();
        let results = match Self::on_archive(move || {
            reader.passages(
                &args.query,
                args.collection_id.as_deref(),
                args.item_id.as_deref(),
                args.limit,
            )
        })
        .await
        {
            Ok(results) => results,
            Err(error) => {
                return Err(self.read_failure(&caller, TOOL_SEARCH_PASSAGES, error, query_chars))
            }
        };
        let mut object_ids: Vec<String> = Vec::new();
        for record in &results.records {
            if !object_ids.contains(&record.item_id) {
                object_ids.push(record.item_id.clone());
            }
        }
        self.audit_ok(&caller, TOOL_SEARCH_PASSAGES, object_ids, query_chars);
        Ok(Json(results))
    }

    #[tool(
        name = "documents_get_text",
        description = "El texto de un ítem por páginas (un archivo o página del documento por entrada), en el orden de sus archivos: texto nativo u OCR, o la transcripción si es audio. Cada entrada trae `textHash` (sha256 del texto completo de la página) y `startChar`/`endChar` en caracteres Unicode, para citar y para saber si el texto cambió. Devuelve a lo sumo `maxChars` caracteres por llamada (1 a 20000, por defecto 4000); seguí con `cursor` (el `nextCursor` anterior). `assetId` lee un solo archivo.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn documents_get_text(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<DocumentsGetTextParams>>,
    ) -> Result<Json<DocumentText>, CallToolResult> {
        let caller = self.guard(&context, TOOL_DOCUMENTS_GET_TEXT)?;
        let args = self.arguments(&caller, TOOL_DOCUMENTS_GET_TEXT, args)?;
        let reader = self.reader.clone();
        let text = match Self::on_archive(move || {
            reader.document_text(
                &args.item_id,
                args.asset_id.as_deref(),
                args.cursor.as_deref(),
                args.max_chars,
            )
        })
        .await
        {
            Ok(text) => text,
            Err(error) => {
                return Err(self.read_failure(&caller, TOOL_DOCUMENTS_GET_TEXT, error, None))
            }
        };
        self.audit_ok(
            &caller,
            TOOL_DOCUMENTS_GET_TEXT,
            vec![text.item.id.clone()],
            None,
        );
        Ok(Json(text))
    }

    #[tool(
        name = "jobs_list",
        description = "Trabajos de procesamiento (lotes de OCR, embeddings, entidades, tripletas o extracción) que la persona preparó o empezó, del más nuevo al más viejo, con su estado y cuántas unidades terminaron, fallaron o siguen activas. Filtra por `states`. Pagina con `cursor` y `limit` (1 a 200, por defecto 50).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jobs_list(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<JobsListParams>>,
    ) -> Result<Json<JobsPage>, CallToolResult> {
        let caller = self.guard(&context, TOOL_JOBS_LIST)?;
        let args = self.arguments(&caller, TOOL_JOBS_LIST, args)?;
        let jobs = self.jobs.clone();
        let page =
            match Self::on_jobs(move || jobs.list(args.states, args.cursor.as_deref(), args.limit))
                .await
            {
                Ok(page) => page,
                Err(error) => return Err(self.jobs_failure(&caller, TOOL_JOBS_LIST, error)),
            };
        let object_ids = page.records.iter().map(|job| job.id.clone()).collect();
        self.audit_ok(&caller, TOOL_JOBS_LIST, object_ids, None);
        Ok(Json(page))
    }

    #[tool(
        name = "jobs_get",
        description = "Un trabajo de procesamiento por su id: estado, operaciones, colecciones, progreso, revisión y una página de sus tareas (filtrables por `taskState` y `taskKind`, paginadas con `tasksCursor` y `tasksLimit`). La `revision` es la que piden las acciones sobre el trabajo.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn jobs_get(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<JobsGetParams>>,
    ) -> Result<Json<JobDetail>, CallToolResult> {
        let caller = self.guard(&context, TOOL_JOBS_GET)?;
        let args = self.arguments(&caller, TOOL_JOBS_GET, args)?;
        let jobs = self.jobs.clone();
        let detail = match Self::on_jobs(move || {
            jobs.get(
                &args.job_id,
                args.task_state.as_deref(),
                args.task_kind.as_deref(),
                args.tasks_cursor.as_deref(),
                args.tasks_limit,
            )
        })
        .await
        {
            Ok(detail) => detail,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_JOBS_GET, error)),
        };
        self.audit_ok(&caller, TOOL_JOBS_GET, vec![detail.job.id.clone()], None);
        Ok(Json(detail))
    }

    #[tool(
        name = "processing_prepare",
        description = "Prepara un trabajo de procesamiento pausado sobre colecciones (`collectionIds`) y/o ítems (`itemIds`): OCR, `embeddings`, entidades (`ner`), tripletas (`triples`), corrección OCR (`ocr_correction`), resúmenes por página (`summary`) o un esquema (`schema:<id>`). Preparar no gasta nada: corrección, resúmenes, entidades, tripletas y esquemas son llamadas pagas a un servicio remoto y empezar el trabajo lo aprueba la persona en EntropIA. `requestId` es tu id para este pedido; reenviarlo igual devuelve el mismo trabajo. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn processing_prepare(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ProcessingPrepareParams>>,
    ) -> Result<Json<PrepareReport>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_PROCESSING_PREPARE,
            Scope::LocalWrite,
            "Este asistente no tiene permiso para preparar procesamiento: la persona puede dárselo en Configuración › Asistentes («Preparar procesamiento y editar el corpus»).",
        )?;
        let args = self.arguments(&caller, TOOL_PROCESSING_PREPARE, args)?;
        let jobs = self.jobs.clone();
        let client_id = caller.client_id.clone();
        let report = match Self::on_jobs(move || {
            jobs.prepare(
                &client_id,
                &args.request_id,
                &args.collection_ids.unwrap_or_default(),
                &args.item_ids.unwrap_or_default(),
                &args.operations,
            )
        })
        .await
        {
            Ok(report) => report,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_PROCESSING_PREPARE, error)),
        };
        self.audit_ok(
            &caller,
            TOOL_PROCESSING_PREPARE,
            vec![report.job_id.clone()],
            None,
        );
        Ok(Json(report))
    }

    #[tool(
        name = "collections_create",
        description = "Crea una colección con `name` y, si querés, `description`. `requestId` es tu id para este pedido: reenviarlo igual devuelve la misma colección (`created` false) en vez de crear otra. Necesita el permiso «Preparar procesamiento y editar el corpus», que también habilita editar el corpus.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn collections_create(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<CollectionsCreateParams>>,
    ) -> Result<Json<CreatedCollection>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_COLLECTIONS_CREATE,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_COLLECTIONS_CREATE, args)?;
        let corpus = self.corpus.clone();
        let client_id = caller.client_id.clone();
        let created = match Self::on_jobs(move || {
            corpus.create_collection(
                &client_id,
                &args.request_id,
                &args.name,
                args.description.as_deref(),
            )
        })
        .await
        {
            Ok(created) => created,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_COLLECTIONS_CREATE, error)),
        };
        self.audit_ok(
            &caller,
            TOOL_COLLECTIONS_CREATE,
            vec![created.id.clone()],
            None,
        );
        Ok(Json(created))
    }

    #[tool(
        name = "collections_update",
        description = "Renombra una colección o cambia su descripción (`null` la borra). `revision` es el `updatedAt` que leíste: si la colección cambió desde entonces, responde revision_conflict y no cambia nada. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn collections_update(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<CollectionsUpdateParams>>,
    ) -> Result<Json<CollectionDto>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_COLLECTIONS_UPDATE,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_COLLECTIONS_UPDATE, args)?;
        let corpus = self.corpus.clone();
        let updated = match Self::on_jobs(move || {
            corpus.update_collection(
                &args.collection_id,
                args.revision,
                &CollectionPatch {
                    name: args.name,
                    description: args.description,
                },
            )
        })
        .await
        {
            Ok(updated) => updated,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_COLLECTIONS_UPDATE, error)),
        };
        self.audit_ok(
            &caller,
            TOOL_COLLECTIONS_UPDATE,
            vec![updated.id.clone()],
            None,
        );
        Ok(Json(updated))
    }

    #[tool(
        name = "collections_delete",
        description = "Pide borrar una colección y todo su contenido: documentos, páginas, notas y anotaciones. No borra nada por sí solo: la persona ve en EntropIA qué se pierde y lo aprueba o rechaza; consultalo con approvals_status. Si la colección ganó o perdió un documento antes de la respuesta, no se borra nada y el pedido termina failed. No se puede deshacer y el borrado llega a todos los equipos. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn collections_delete(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<CollectionsDeleteParams>>,
    ) -> Result<Json<Approval>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_COLLECTIONS_DELETE,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_COLLECTIONS_DELETE, args)?;
        let corpus = self.corpus.clone();
        let approvals = self.approvals.clone();
        let collection_id = args.collection_id.clone();
        let (client_id, client_name) = (caller.client_id.clone(), caller.name.clone());
        let approval = match Self::on_jobs(move || {
            corpus.request_collection_deletion(&approvals, &client_id, &client_name, &collection_id)
        })
        .await
        {
            Ok(approval) => approval,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_COLLECTIONS_DELETE, error)),
        };
        self.audit_ok(
            &caller,
            TOOL_COLLECTIONS_DELETE,
            vec![args.collection_id, approval.id.clone()],
            None,
        );
        Ok(Json(approval))
    }

    #[tool(
        name = "items_update",
        description = "Cambia el título de un ítem o sus campos de metadatos (`metadata` reemplaza todos los campos; los valores son texto). `revision` es el `updatedAt` que leíste: si el ítem cambió, responde revision_conflict y no cambia nada. Los datos internos de EntropIA se conservan y no se pueden escribir. El cambio queda buscable enseguida. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn items_update(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ItemsUpdateParams>>,
    ) -> Result<Json<UpdatedItem>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_ITEMS_UPDATE,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_ITEMS_UPDATE, args)?;
        let corpus = self.corpus.clone();
        let updated = match Self::on_jobs(move || {
            corpus.update_item(&args.item_id, args.revision, args.title, args.metadata)
        })
        .await
        {
            Ok(updated) => updated,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_ITEMS_UPDATE, error)),
        };
        self.audit_ok(&caller, TOOL_ITEMS_UPDATE, vec![updated.id.clone()], None);
        Ok(Json(updated))
    }

    #[tool(
        name = "documents_text_update",
        description = "Pide reemplazar el texto reconocido (nativo u OCR) de una página por `text`, el texto completo nuevo. `textHash` es el de documents_get_text: si la página cambió, responde revision_conflict. No cambia nada por sí solo: la persona ve una vista previa en EntropIA y lo aprueba o rechaza; consultalo con approvals_status. Si se aplica, el texto anterior queda guardado y «Restaurar original» lo deshace. No sirve para transcripciones de audio. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn documents_text_update(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<DocumentsTextUpdateParams>>,
    ) -> Result<Json<Approval>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_DOCUMENTS_TEXT_UPDATE,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_DOCUMENTS_TEXT_UPDATE, args)?;
        let corpus = self.corpus.clone();
        let approvals = self.approvals.clone();
        let (client_id, client_name) = (caller.client_id.clone(), caller.name.clone());
        let approval = match Self::on_jobs(move || {
            corpus.request_text_replacement(
                &approvals,
                &client_id,
                &client_name,
                &TextReplacement {
                    item_id: args.item_id,
                    asset_id: args.asset_id,
                    text_hash: args.text_hash,
                    text: args.text,
                },
            )
        })
        .await
        {
            Ok(approval) => approval,
            Err(error) => {
                return Err(self.jobs_failure(&caller, TOOL_DOCUMENTS_TEXT_UPDATE, error))
            }
        };
        let object_ids = match &approval.action {
            ApprovalAction::ReplaceText { item_id, .. } => {
                vec![item_id.clone(), approval.id.clone()]
            }
            _ => vec![approval.id.clone()],
        };
        self.audit_ok(&caller, TOOL_DOCUMENTS_TEXT_UPDATE, object_ids, None);
        Ok(Json(approval))
    }

    #[tool(
        name = "import_sources_list",
        description = "Las carpetas de entrada que la persona habilitó en Configuración › Asistentes y los archivos que se pueden importar de cada una (PDF, imágenes y audio, sin subcarpetas; hasta 500 por carpeta). No cambia nada. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn import_sources_list(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ImportSourcesListParams>>,
    ) -> Result<Json<ImportSources>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_IMPORT_SOURCES_LIST,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        self.arguments(&caller, TOOL_IMPORT_SOURCES_LIST, args)?;
        let corpus = self.corpus.clone();
        let folders = self.access.import_folders();
        let sources = match Self::on_jobs(move || corpus.import_sources(&folders)).await {
            Ok(sources) => sources,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_IMPORT_SOURCES_LIST, error)),
        };
        self.audit_ok(&caller, TOOL_IMPORT_SOURCES_LIST, Vec::new(), None);
        Ok(Json(sources))
    }

    #[tool(
        name = "documents_import",
        description = "Pide importar archivos de una carpeta de entrada (`folder`, como la devuelve import_sources_list) a una colección: un ítem por archivo, y cada PDF dividido en páginas. No importa nada por sí solo: la persona ve la lista de archivos en EntropIA y lo aprueba o rechaza; consultalo con approvals_status, que al final dice qué se importó y qué se salteó (un archivo que cambió, que ya estaba en la colección o que dejó de estar habilitado). Repetir el mismo pedido devuelve el mismo. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn documents_import(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<DocumentsImportParams>>,
    ) -> Result<Json<Approval>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_DOCUMENTS_IMPORT,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_DOCUMENTS_IMPORT, args)?;
        let corpus = self.corpus.clone();
        let approvals = self.approvals.clone();
        let folders = self.access.import_folders();
        let (client_id, client_name) = (caller.client_id.clone(), caller.name.clone());
        let collection_id = args.collection_id.clone();
        let approval = match Self::on_jobs(move || {
            corpus.request_import(
                &approvals,
                &client_id,
                &client_name,
                &folders,
                &ImportRequest {
                    collection_id: args.collection_id,
                    folder: args.folder,
                    files: args.files,
                },
            )
        })
        .await
        {
            Ok(approval) => approval,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_DOCUMENTS_IMPORT, error)),
        };
        self.audit_ok(
            &caller,
            TOOL_DOCUMENTS_IMPORT,
            vec![collection_id, approval.id.clone()],
            None,
        );
        Ok(Json(approval))
    }

    #[tool(
        name = "documents_delete",
        description = "Pide borrar documentos con todo lo que contienen: archivos, páginas, notas y anotaciones. No borra nada por sí solo: la persona ve en EntropIA cuántos documentos, páginas, notas y anotaciones se pierden y lo aprueba o rechaza; consultalo con approvals_status. El pedido queda atado a la revisión de cada documento: si uno cambió antes de la respuesta, no se borra nada y el pedido termina failed. No se puede deshacer y el borrado llega a todos los equipos. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn documents_delete(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<DocumentsDeleteParams>>,
    ) -> Result<Json<Approval>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_DOCUMENTS_DELETE,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_DOCUMENTS_DELETE, args)?;
        let corpus = self.corpus.clone();
        let approvals = self.approvals.clone();
        let (client_id, client_name) = (caller.client_id.clone(), caller.name.clone());
        let approval = match Self::on_jobs(move || {
            corpus.request_items_deletion(&approvals, &client_id, &client_name, &args.item_ids)
        })
        .await
        {
            Ok(approval) => approval,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_DOCUMENTS_DELETE, error)),
        };
        let mut object_ids = match &approval.action {
            ApprovalAction::DeleteItems { item_ids, .. } => item_ids.clone(),
            _ => Vec::new(),
        };
        object_ids.push(approval.id.clone());
        self.audit_ok(&caller, TOOL_DOCUMENTS_DELETE, object_ids, None);
        Ok(Json(approval))
    }

    #[tool(
        name = "documents_index",
        description = "Vuelve a indexar un ítem para la búsqueda (`search_items`) con su título, metadatos y texto actuales. Las ediciones hechas con las herramientas ya lo hacen solas; sirve cuando el índice quedó atrasado. No gasta. Necesita el permiso «Preparar procesamiento y editar el corpus».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn documents_index(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<DocumentsIndexParams>>,
    ) -> Result<Json<IndexedItem>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_DOCUMENTS_INDEX,
            Scope::LocalWrite,
            REFUSE_LOCAL_WRITE,
        )?;
        let args = self.arguments(&caller, TOOL_DOCUMENTS_INDEX, args)?;
        let corpus = self.corpus.clone();
        let indexed = match Self::on_jobs(move || corpus.reindex(&args.item_id)).await {
            Ok(indexed) => indexed,
            Err(error) => return Err(self.jobs_failure(&caller, TOOL_DOCUMENTS_INDEX, error)),
        };
        self.audit_ok(
            &caller,
            TOOL_DOCUMENTS_INDEX,
            vec![indexed.item_id.clone()],
            None,
        );
        Ok(Json(indexed))
    }

    #[tool(
        name = "processing_start",
        description = "Pide empezar un trabajo preparado (state \"ready\") en la `revision` que devolvió jobs_get. No empieza nada por sí solo: la persona lo aprueba o rechaza en EntropIA, que se abre para mostrarle el pedido. Devuelve el pedido (`id`, `status` pending); consultalo con approvals_status. Repetir el mismo pedido devuelve el mismo. Necesita el permiso «Pedir procesamiento pago».",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn processing_start(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ProcessingStartParams>>,
    ) -> Result<Json<Approval>, CallToolResult> {
        let caller = self.guard_scope(
            &context,
            TOOL_PROCESSING_START,
            Scope::RemoteSpend,
            REFUSE_REMOTE_SPEND,
        )?;
        let args = self.arguments(&caller, TOOL_PROCESSING_START, args)?;
        let approval = self
            .ask_owner(
                &caller,
                TOOL_PROCESSING_START,
                ApprovalAction::Start,
                args.job_id,
                args.revision,
            )
            .await?;
        Ok(Json(approval))
    }

    #[tool(
        name = "processing_control",
        description = "Controla un trabajo. `pause` y `cancel` se aplican enseguida y no gastan: las unidades en curso terminan su paso y lo ya procesado se conserva (permiso «Preparar procesamiento y editar el corpus»). `resume` y `retry` vuelven a gastar, así que piden la aprobación de la persona en EntropIA y necesitan `revision` (permiso «Pedir procesamiento pago»); `retry` reabre una tarea (`taskId`) o todas las fallidas (`failedOnly`: true).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn processing_control(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ProcessingControlParams>>,
    ) -> Result<Json<ControlReport>, CallToolResult> {
        let caller = Self::caller(&context)?;
        let args = self.arguments(&caller, TOOL_PROCESSING_CONTROL, args)?;
        let tool = TOOL_PROCESSING_CONTROL;
        match args.action.as_str() {
            "pause" | "cancel" => {
                if !caller.has(Scope::LocalWrite) {
                    return Err(self.refuse_scope(&caller, tool, REFUSE_LOCAL_WRITE));
                }
                let jobs = self.jobs.clone();
                let (job_id, action, revision) =
                    (args.job_id.clone(), args.action.clone(), args.revision);
                let job = match Self::on_jobs(move || jobs.control_now(&job_id, &action, revision))
                    .await
                {
                    Ok(job) => job,
                    Err(error) => return Err(self.jobs_failure(&caller, tool, error)),
                };
                self.audit_ok(&caller, tool, vec![job.id.clone()], None);
                Ok(Json(ControlReport {
                    job_id: job.id.clone(),
                    action: args.action,
                    outcome: "done".to_string(),
                    job: Some(job),
                    approval: None,
                }))
            }
            "resume" | "retry" => {
                if !caller.has(Scope::RemoteSpend) {
                    return Err(self.refuse_scope(&caller, tool, REFUSE_REMOTE_SPEND));
                }
                let Some(revision) = args.revision else {
                    return Err(self.refuse_argument(
                        &caller,
                        tool,
                        "`resume` y `retry` necesitan `revision` (la que devolvió jobs_get): la persona aprueba exactamente ese estado.",
                    ));
                };
                let action = if args.action == "resume" {
                    ApprovalAction::Resume
                } else {
                    let failed_only = args.failed_only.unwrap_or(false);
                    if args.task_id.is_none() && !failed_only {
                        return Err(self.refuse_argument(
                            &caller,
                            tool,
                            "`retry` necesita `taskId` o `failedOnly`: true.",
                        ));
                    }
                    ApprovalAction::Retry {
                        task_id: args.task_id.clone(),
                        failed_only,
                    }
                };
                let approval = self
                    .ask_owner(&caller, tool, action, args.job_id.clone(), revision)
                    .await?;
                Ok(Json(ControlReport {
                    job_id: args.job_id,
                    action: args.action,
                    outcome: "approval_pending".to_string(),
                    job: None,
                    approval: Some(approval),
                }))
            }
            other => Err(self.refuse_argument(
                &caller,
                tool,
                &format!("Acción desconocida \"{other}\": usá pause, cancel, resume o retry."),
            )),
        }
    }

    #[tool(
        name = "approvals_status",
        description = "Estado de un pedido de aprobación de este asistente: pending (esperando a la persona), approved (aprobado y ejecutado), failed (aprobado pero el trabajo había cambiado; no se ejecutó nada), rejected o expired (vence a los 10 minutos).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn approvals_status(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<Strict<ApprovalsStatusParams>>,
    ) -> Result<Json<Approval>, CallToolResult> {
        let caller = self.guard(&context, TOOL_APPROVALS_STATUS)?;
        let args = self.arguments(&caller, TOOL_APPROVALS_STATUS, args)?;
        match self
            .approvals
            .get_for_client(&caller.client_id, &args.approval_id)
        {
            Some(approval) => {
                self.audit_ok(
                    &caller,
                    TOOL_APPROVALS_STATUS,
                    vec![approval.id.clone()],
                    None,
                );
                Ok(Json(approval))
            }
            None => {
                self.audit_error(&caller, TOOL_APPROVALS_STATUS, ERROR_NOT_FOUND, None);
                Err(tool_error(
                    ERROR_NOT_FOUND,
                    format!("No hay un pedido {} de este asistente.", args.approval_id),
                ))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgentServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, app_version()))
    }
}
