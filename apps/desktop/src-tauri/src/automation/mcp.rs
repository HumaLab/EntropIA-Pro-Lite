//! What the endpoint offers over MCP.
//!
//! One [`AgentServer`] is built per request (the transport runs stateless), so
//! nothing outlives a request and no identity is cached between them: every
//! tool reads the caller from the `Principal` the endpoint's guard put into the
//! HTTP request (D6).
//!
//! `entropia_status` is the only tool in this release. T3 adds the read-only
//! domain tools here — one `#[tool]` method per intent, each taking the
//! [`RequestContext`] and reading [`Principal`] from it, is all the router and
//! `tools/list` need.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Json},
    model::{Implementation, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::automation::access::{Principal, Scope};
use crate::automation::server::{app_version, profile_name, variant_name, PROTOCOL};

/// Name the endpoint answers `initialize` with.
pub const SERVER_NAME: &str = "entropia";

/// `entropia_status`'s payload.
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
}

/// The authenticated caller, without its token.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CallerInfo {
    pub id: String,
    pub name: String,
    pub scopes: Vec<Scope>,
}

#[derive(Clone)]
pub struct AgentServer {
    tool_router: ToolRouter<Self>,
}

impl AgentServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// The caller the endpoint authenticated for this request.
    ///
    /// The transport copies [`http::request::Parts`] into the tool context and
    /// the guard put the [`Principal`] in those parts' extensions. A missing
    /// principal means the request bypassed the guard, which is a server bug:
    /// no tool may fall back to "some client".
    fn caller(context: &RequestContext<RoleServer>) -> Result<Principal, ErrorData> {
        context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>())
            .cloned()
            .ok_or_else(|| {
                ErrorData::internal_error(
                    "the request reached the tool without an authenticated client",
                    None,
                )
            })
    }
}

impl Default for AgentServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl AgentServer {
    #[tool(
        name = "entropia_status",
        description = "Estado de esta instancia de EntropIA y del asistente conectado: versión, variante, perfil e identidad del cliente."
    )]
    async fn entropia_status(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<StatusReport>, ErrorData> {
        let caller = Self::caller(&context)?;
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
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgentServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, app_version()))
    }
}
