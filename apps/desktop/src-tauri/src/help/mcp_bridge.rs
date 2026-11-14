//! In-process MCP bridge for the in-app help assistant (T21, S15, D11).
//!
//! The help agent drives the SAME agent MCP server in-process: an rmcp client
//! over an in-memory duplex talks to an `AgentServer` built exactly like the
//! app builds it (archive path, cache dir, the shared approval store and
//! corpus events), with the synthetic «Ayuda» principal injected into every
//! request's extensions on the server side. Lazily created once per process
//! (like the help index); independent of «Permitir asistentes externos».

// Not wired yet (T22/T24 own the gate and the panel); only tests call it.
#![cfg_attr(not(test), allow(dead_code))]

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ClientNotification, ClientRequest, ServerConfig,
        ServerResult, Tool,
    },
    service::{NotificationContext, RequestContext, RunningService, Service},
    ErrorData as McpError, RoleClient, RoleServer, ServiceExt,
};

use super::agent_llm::{HelpToolExecutor, ToolOutcome, ToolSpec};
use crate::automation::access::{AgentAccess, Principal};
use crate::automation::approvals::ApprovalStore;
use crate::automation::corpus_tools::CorpusEvents;
use crate::automation::mcp::AgentServer;

/// The server side of the bridge: the real `AgentServer` with the synthetic
/// «Ayuda» principal inserted into every request's extensions before it
/// dispatches. Extensions never cross the duplex, so this wrapper is the only
/// way the principal gets in — and only in-process code holds this wrapper.
struct InProcessHelpServer {
    inner: AgentServer,
    principal: Principal,
}

impl Service<RoleServer> for InProcessHelpServer {
    async fn handle_request(
        &self,
        request: ClientRequest,
        mut context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, McpError> {
        context.extensions.insert(self.principal.clone());
        <AgentServer as Service<RoleServer>>::handle_request(&self.inner, request, context).await
    }

    async fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> Result<(), McpError> {
        <AgentServer as Service<RoleServer>>::handle_notification(
            &self.inner,
            notification,
            context,
        )
        .await
    }

    fn get_info(&self) -> ServerConfig {
        <AgentServer as Service<RoleServer>>::get_info(&self.inner)
    }
}

/// The tool the bridge exposes to the help agent loop.
fn tool_spec_from(tool: &Tool) -> ToolSpec {
    ToolSpec {
        name: tool.name.to_string(),
        description: tool
            .description
            .as_ref()
            .map(|description| description.to_string())
            .unwrap_or_default(),
        input_schema: serde_json::Value::Object(tool.input_schema.as_ref().clone()),
    }
}

/// The text the model sees for an `Ok` result: the text blocks in order, or
/// the structured payload as JSON when the tool answered with data only.
fn result_text(result: &CallToolResult) -> String {
    let mut parts = Vec::new();
    for block in &result.content {
        if let Some(text) = block.as_text() {
            parts.push(text.text.clone());
        }
    }
    let joined = parts.join("\n");
    if !joined.trim().is_empty() {
        return joined;
    }
    result
        .structured_content
        .as_ref()
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_default()
}

/// Maps one MCP result to the agent outcome: tool errors become `Failed`,
/// except the scope refusal, which becomes `Denied` so the loop (and T22's
/// gate) can tell "not allowed" from "broke".
fn outcome_from_result(result: &CallToolResult) -> ToolOutcome {
    if result.is_error.unwrap_or(false) {
        let body = result.structured_content.as_ref();
        let code = body
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str())
            .unwrap_or("error");
        let message = body
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| result_text(result));
        if code == "forbidden_scope" {
            return ToolOutcome::Denied { reason: message };
        }
        return ToolOutcome::Failed { message };
    }
    ToolOutcome::Ok {
        content: result_text(result),
    }
}

/// The in-process bridge: an rmcp client over a duplex to the wrapped
/// `AgentServer`, with the tool list fetched once at start.
pub(crate) struct HelpMcpBridge {
    client: RunningService<RoleClient, ()>,
    _server: RunningService<RoleServer, InProcessHelpServer>,
    tools: Vec<ToolSpec>,
}

impl HelpMcpBridge {
    /// Starts the pair: the wrapped server on one end of the duplex, the
    /// client on the other. `approvals` and `corpus_events` are the SAME
    /// shared instances the app runs, so approvals reach the existing dialog
    /// and corpus changes notify the UI.
    pub(crate) async fn start(
        db_path: PathBuf,
        cache_dir: PathBuf,
        approvals: Arc<ApprovalStore>,
        corpus_events: Arc<CorpusEvents>,
        access: Arc<AgentAccess>,
    ) -> Result<Self, String> {
        let service = InProcessHelpServer {
            inner: AgentServer::new(db_path, cache_dir, approvals, corpus_events, access),
            principal: Principal::in_app_help(),
        };
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move { service.serve(server_io).await });
        let client = ()
            .serve(client_io)
            .await
            .map_err(|error| format!("No se pudo iniciar el puente MCP de ayuda: {error}"))?;
        let server = server_task
            .await
            .map_err(|error| format!("No se pudo iniciar el puente MCP de ayuda: {error}"))?
            .map_err(|error| format!("No se pudo iniciar el puente MCP de ayuda: {error}"))?;
        let tools = client
            .list_all_tools()
            .await
            .map_err(|error| format!("No se pudo leer las herramientas de ayuda: {error}"))?
            .iter()
            .map(tool_spec_from)
            .collect();
        Ok(Self {
            client,
            _server: server,
            tools,
        })
    }

    /// The cached tool list, in `tools/list` order.
    pub(crate) fn list_tools(&self) -> Vec<ToolSpec> {
        self.tools.clone()
    }

    /// Calls one tool by name. A non-object `args` never reaches the server;
    /// a transport failure and an MCP tool error both become `Failed`.
    pub(crate) async fn call_tool(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
        let arguments = match args {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            _ => {
                return ToolOutcome::Failed {
                    message: "Los argumentos de la herramienta no son válidos: se esperaba un objeto JSON".to_string(),
                };
            }
        };
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = arguments;
        match self.client.call_tool(params).await {
            Ok(result) => outcome_from_result(&result),
            Err(error) => ToolOutcome::Failed {
                message: format!("La herramienta falló: {error}"),
            },
        }
    }
}

impl HelpToolExecutor for HelpMcpBridge {
    fn tools(&self) -> Vec<ToolSpec> {
        self.list_tools()
    }

    async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
        self.call_tool(name, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::super::agent_llm::ToolOutcome;
    use super::*;
    use crate::automation::access::tests::MemorySecrets;
    use crate::automation::access::{AgentAccess, Scope};
    use crate::automation::approvals::ApprovalStore;
    use crate::automation::corpus_tools::CorpusEvents;
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// The 22 tools D10 serves, in `tools/list` order (mirrors the HTTP
    /// endpoint's list in `automation::server::tests`).
    const EXPECTED_TOOLS: [&str; 22] = [
        "approvals_status",
        "collections_create",
        "collections_delete",
        "collections_list",
        "collections_update",
        "documents_delete",
        "documents_get_text",
        "documents_import",
        "documents_index",
        "documents_text_update",
        "entropia_status",
        "import_sources_list",
        "items_get",
        "items_list",
        "items_update",
        "jobs_get",
        "jobs_list",
        "processing_control",
        "processing_prepare",
        "processing_start",
        "search_items",
        "search_passages",
    ];

    async fn bridge() -> (tempfile::TempDir, HelpMcpBridge) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = crate::automation::test_archive::seeded_archive(dir.path());
        let cache_dir = dir.path().join("cache");
        let approvals = Arc::new(ApprovalStore::default());
        let corpus_events = Arc::new(CorpusEvents::default());
        let secrets = Arc::new(MemorySecrets::default());
        let access = Arc::new(AgentAccess::load(dir.path(), secrets));
        let bridge = HelpMcpBridge::start(db_path, cache_dir, approvals, corpus_events, access)
            .await
            .expect("the bridge starts against a seeded archive");
        (dir, bridge)
    }

    fn audit_lines(dir: &tempfile::TempDir) -> Vec<Value> {
        let path: PathBuf = dir
            .path()
            .join("cache")
            .join("logs")
            .join(crate::automation::audit::AUDIT_FILE);
        std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every audit line is JSON"))
            .collect()
    }

    #[tokio::test]
    async fn list_tools_returns_the_same_tool_names_the_http_server_exposes() {
        let (_dir, bridge) = bridge().await;
        let mut names: Vec<String> = bridge
            .list_tools()
            .iter()
            .map(|tool| tool.name.clone())
            .collect();
        names.sort();
        let mut expected: Vec<String> =
            EXPECTED_TOOLS.iter().map(|name| name.to_string()).collect();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[tokio::test]
    async fn list_tools_carries_descriptions_and_strict_input_schemas() {
        let (_dir, bridge) = bridge().await;
        let tools = bridge.list_tools();
        assert_eq!(tools.len(), EXPECTED_TOOLS.len());
        for tool in &tools {
            assert!(
                !tool.description.trim().is_empty(),
                "{} carries a description for the model",
                tool.name
            );
            assert_eq!(
                tool.input_schema.get("type"),
                Some(&Value::String("object".to_string())),
                "{} publishes an object schema",
                tool.name
            );
            assert_eq!(
                tool.input_schema.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "{} refuses unknown fields (D10)",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn a_read_tool_works_through_the_bridge() {
        let (_dir, bridge) = bridge().await;
        match bridge.call_tool("collections_list", json!({})).await {
            ToolOutcome::Ok { content } => {
                assert!(
                    content.contains("Alfa"),
                    "the seeded collection is listed: {content}"
                );
            }
            other => panic!("collections_list must succeed: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_local_write_tool_creates_a_collection() {
        let (_dir, bridge) = bridge().await;
        match bridge
            .call_tool(
                "collections_create",
                json!({
                    "requestId": "ayuda-1",
                    "name": "Colección de Ayuda",
                    "description": "creada por el puente",
                }),
            )
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(
                    content.contains("Colección de Ayuda"),
                    "the created collection is reported: {content}"
                );
            }
            other => panic!("collections_create must succeed: {other:?}"),
        }
        match bridge.call_tool("collections_list", json!({})).await {
            ToolOutcome::Ok { content } => {
                assert!(
                    content.contains("Colección de Ayuda"),
                    "the collection persists: {content}"
                );
            }
            other => panic!("collections_list must succeed: {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_approval_tool_returns_pending_and_names_ayuda() {
        let (_dir, bridge) = bridge().await;
        match bridge
            .call_tool("collections_delete", json!({ "collectionId": "col_beta" }))
            .await
        {
            ToolOutcome::Ok { content } => {
                let approval: Value = serde_json::from_str(&content).expect("the approval is JSON");
                assert_eq!(approval["status"], "pending");
                assert_eq!(approval["clientName"], "Ayuda");
                assert_eq!(approval["clientId"], "in-app-help");
            }
            other => panic!("collections_delete must ask, not execute: {other:?}"),
        }
        // Nothing was deleted: the approval waits for the owner.
        match bridge
            .call_tool("items_list", json!({ "collectionId": "col_beta" }))
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(
                    content.contains("Revista"),
                    "the collection is untouched until approval: {content}"
                );
            }
            other => panic!("items_list must succeed: {other:?}"),
        }
    }

    #[tokio::test]
    async fn calls_through_the_bridge_are_audited_as_ayuda() {
        let (dir, bridge) = bridge().await;
        match bridge.call_tool("collections_list", json!({})).await {
            ToolOutcome::Ok { .. } => {}
            other => panic!("collections_list must succeed: {other:?}"),
        }
        let lines = audit_lines(&dir);
        assert_eq!(lines.len(), 1, "one line per call: {lines:?}");
        assert_eq!(lines[0]["tool"], "collections_list");
        assert_eq!(lines[0]["status"], "ok");
        assert_eq!(lines[0]["clientName"], "Ayuda");
        assert_eq!(lines[0]["clientId"], "in-app-help");
    }

    #[tokio::test]
    async fn an_unknown_tool_name_is_a_failure_not_a_crash() {
        let (_dir, bridge) = bridge().await;
        match bridge.call_tool("no_existe", json!({})).await {
            ToolOutcome::Failed { message } => {
                assert!(!message.trim().is_empty(), "the failure says something");
            }
            other => panic!("an unknown tool must fail: {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_synthetic_principal_carries_the_help_identity_and_scopes() {
        let principal = crate::automation::access::Principal::in_app_help();
        assert_eq!(principal.client_id, "in-app-help");
        assert_eq!(principal.name, "Ayuda");
        assert!(principal.has(Scope::Read));
        assert!(principal.has(Scope::LocalWrite));
        assert!(principal.has(Scope::RemoteSpend));
    }

    #[test]
    fn outcome_mapping_sends_scope_refusals_to_denied_and_errors_to_failed() {
        let denied = rmcp::model::CallToolResult::structured_error(
            serde_json::json!({ "error": "forbidden_scope", "message": "sin permiso" }),
        );
        match outcome_from_result(&denied) {
            ToolOutcome::Denied { reason } => assert!(reason.contains("sin permiso")),
            other => panic!("forbidden_scope must be Denied: {other:?}"),
        }
        let failed = rmcp::model::CallToolResult::structured_error(
            serde_json::json!({ "error": "not_found", "message": "no existe" }),
        );
        match outcome_from_result(&failed) {
            ToolOutcome::Failed { message } => assert!(message.contains("no existe")),
            other => panic!("not_found must be Failed: {other:?}"),
        }
    }

    #[test]
    fn outcome_mapping_joins_text_content_for_ok_results() {
        let ok = rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::text("hola"),
            rmcp::model::ContentBlock::text("mundo"),
        ]);
        match outcome_from_result(&ok) {
            ToolOutcome::Ok { content } => {
                assert!(
                    content.contains("hola") && content.contains("mundo"),
                    "{content}"
                );
            }
            other => panic!("text content must be Ok: {other:?}"),
        }
    }

    /// A request with neither a direct principal nor the guard's HTTP parts
    /// — exactly what a remote call that bypassed the guard looks like — is
    /// still rejected: the new in-process path leaks nothing to HTTP.
    #[tokio::test]
    async fn a_request_without_any_principal_is_still_rejected() {
        use rmcp::ServiceExt as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = crate::automation::test_archive::seeded_archive(dir.path());
        let server = crate::automation::mcp::AgentServer::new(
            db_path,
            dir.path().join("cache"),
            Arc::new(ApprovalStore::default()),
            Arc::new(CorpusEvents::default()),
            Arc::new(AgentAccess::load(
                dir.path(),
                Arc::new(MemorySecrets::default()),
            )),
        );
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move { server.serve(server_io).await });
        let client = ().serve(client_io).await.expect("handshake");
        let server = server_task.await.expect("spawn").expect("serve");
        let result = client
            .call_tool(rmcp::model::CallToolRequestParams::new("collections_list"))
            .await
            .expect("the server answers");
        assert!(
            result.is_error.unwrap_or(false),
            "a call without a principal must be refused"
        );
        let code = result
            .structured_content
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str());
        assert_eq!(code, Some("internal_error"));
        let _ = server;
    }
}
