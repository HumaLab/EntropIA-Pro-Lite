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

use crate::automation::access::{Principal, Scope};
use crate::automation::audit::{AuditLine, AuditLog};
use crate::automation::read::{
    ArchiveCounts, ArchiveReader, CollectionsPage, ItemDetail, ItemsPage, ReadError, SearchResults,
};
use crate::automation::server::{app_version, profile_name, variant_name, PROTOCOL};

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
    audit: AuditLog,
}

impl AgentServer {
    /// `db_path` is the archive the tools read; `cache_dir` receives the audit
    /// trail under `logs/`.
    pub fn new(db_path: impl Into<PathBuf>, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            tool_router: Self::tool_router(),
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
        let caller = Self::caller(context)?;
        if caller.has(Scope::Read) {
            return Ok(caller);
        }
        self.audit_error(&caller, tool, ERROR_FORBIDDEN_SCOPE, None);
        Err(tool_error(
            ERROR_FORBIDDEN_SCOPE,
            "Este asistente no tiene permiso de lectura en EntropIA: pedile a la persona que lo emparejó que revise sus permisos."
                .to_string(),
        ))
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
        description = "Un ítem por su id: título, colección, metadatos, revisión y la lista de archivos adjuntos. No devuelve el texto completo: para eso está `search_items`.",
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
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgentServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, app_version()))
    }
}
