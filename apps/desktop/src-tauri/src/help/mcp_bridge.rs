//! In-process MCP bridge for the in-app help assistant (T21, S15, D11).
//!
//! The help agent drives the SAME agent MCP server in-process: an rmcp client
//! over an in-memory duplex talks to an `AgentServer` built exactly like the
//! app builds it (archive path, cache dir, the shared approval store and
//! corpus events), with the synthetic «Ayuda» principal injected into every
//! request's extensions on the server side. Lazily created once per process
//! (like the help index); independent of «Permitir asistentes externos».

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use regex::Regex;

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
/// gate) can tell "not allowed" from "broke". Every text that reaches
/// the model is scrubbed first (D18): file-system paths become the file
/// name or `[carpeta]`, so the model never sees where the user's files live.
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
            return scrub_outcome(ToolOutcome::Denied { reason: message });
        }
        return scrub_outcome(ToolOutcome::Failed { message });
    }
    scrub_outcome(ToolOutcome::Ok {
        content: result_text(result),
    })
}

/// D18: file-system paths in tool results never reach the model. Applied in
/// the help layer (the bridge result -> gate path), not in the MCP server,
/// so external assistants keep today's payloads.
///
/// Path-detection rule: a string value is scrubbed when its key holds a
/// path (`path` or any key ending in `path`, plus `folder`, `root`, `dir`,
/// `directory` and any key ending in `folder`, `directory` or `_dir`) or
/// when the value itself looks like an absolute path (a Windows drive
/// `C:\…` or `C:/…`, a UNC `\\…`, a Unix `/…` or a home `~/…`, plus
/// `file://…` URLs). A path-like key whose value has no slash is left
/// alone (it is not a path). `id` keys (`id`, `*_id`, `*Id`), `title` keys
/// (`title`, `*Title`, `*title(s)`) and `name` keys (`name`, `*Name`) never
/// travel scrubbed: IDs, titles and file names stay verbatim, so a model
/// slug like `google/gemma` (relative, no drive/root) is untouched too.
/// A file (last segment with an extension) keeps only its file name; a
/// folder (no extension, or any folder-named key) becomes `[carpeta]`.
/// JSON payloads are walked key by key; any other text (plain-text tool
/// results, or free prose inside a JSON string) gets the same file-or-folder
/// replacement for every absolute-path substring, via the same patterns the
/// diagnostics scrub uses (drive, UNC, Unix, home, `file://`).
const FOLDER_MARKER: &str = "[carpeta]";

fn is_excluded_key(key: &str) -> bool {
    let low = key.to_lowercase();
    if low == "id" || low == "title" || low == "name" {
        return true;
    }
    if low.ends_with("_id") || key.ends_with("Id") || key.ends_with("ID") {
        return true;
    }
    if low.ends_with("_title")
        || low.ends_with("title")
        || low.ends_with("titles")
        || key.ends_with("Title")
        || key.ends_with("Titles")
    {
        return true;
    }
    if low.ends_with("_name")
        || low.ends_with("name")
        || low.ends_with("names")
        || key.ends_with("Name")
        || key.ends_with("Names")
    {
        return true;
    }
    false
}

fn is_folder_key(key: &str) -> bool {
    let low = key.to_lowercase();
    if matches!(low.as_str(), "folder" | "root" | "dir" | "directory") {
        return true;
    }
    low.ends_with("folder")
        || low.ends_with("directory")
        || low.ends_with("dir")
        || low.ends_with("root")
}

fn is_path_key(key: &str) -> bool {
    if is_folder_key(key) {
        return true;
    }
    let low = key.to_lowercase();
    low == "path" || low.ends_with("path") || low == "file"
}

fn looks_like_absolute_path(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("file://") {
        return true;
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    if trimmed.starts_with("\\\\") {
        return true;
    }
    if trimmed.starts_with('/') {
        return true;
    }
    if trimmed == "~" || trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        return true;
    }
    false
}

fn has_extension(file_name: &str) -> bool {
    let Some(dot) = file_name.rfind('.') else {
        return false;
    };
    let (prefix, suffix) = file_name.split_at(dot);
    let suffix = &suffix[1..];
    !prefix.is_empty()
        && !suffix.is_empty()
        && suffix.len() <= 5
        && suffix.chars().all(|c| c.is_ascii_alphanumeric())
}

fn file_name_of(path: &str) -> String {
    let mut work = path.trim();
    if let Some(rest) = work.strip_prefix("file://") {
        // `file:///Users/ana/acta.pdf` or `file://host/ruta`: the path is
        // what follows the authority; a bare `file://x` has no file name.
        if let Some(slash) = rest.find('/') {
            work = &rest[slash + 1..];
        } else {
            return String::new();
        }
    }
    work = work.trim_matches(|c| c == '/' || c == '\\');
    if work.is_empty() {
        return String::new();
    }
    work.split(['/', '\\'])
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("")
        .trim_end_matches(['.', ' '])
        .to_string()
}

fn scrub_one_path_value(key: &str, value: &str) -> String {
    if is_folder_key(key) {
        return FOLDER_MARKER.to_string();
    }
    let name = file_name_of(value);
    if name.is_empty() {
        return FOLDER_MARKER.to_string();
    }
    if has_extension(&name) {
        name
    } else {
        FOLDER_MARKER.to_string()
    }
}

struct PathPatterns {
    file_url: Regex,
    windows: Regex,
    unc: Regex,
    unix: Regex,
}

fn path_patterns() -> &'static PathPatterns {
    static PATTERNS: OnceLock<PathPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| PathPatterns {
        file_url: Regex::new(r#"file://[^\s"'«»‘’\n]*"#).expect("file url pattern"),
        windows: Regex::new(r#"[A-Za-z]:[\\/][^\s"'«»‘’\n]*"#).expect("windows pattern"),
        unc: Regex::new(r#"\\\\[^\s"'«»‘’\n]*"#).expect("unc pattern"),
        // Unix paths start at a boundary (start, whitespace or a small set
        // of separators) so `google/gemma` never matches: its slash follows
        // a letter, not a boundary. The prefix is kept in the replacement.
        unix: Regex::new(r#"(^|[\s(=:,])(~?/(?:[^/\s"'«»\n]+/)*[^/\s"'«»\n]*)"#)
            .expect("unix pattern"),
    })
}

fn scrub_match(raw: &str) -> String {
    // Keep sentence punctuation outside the path: `acta.pdf.` still reads as
    // the file `acta.pdf`.
    let trimmed = raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'']);
    let trail: String = raw[trimmed.len()..].to_string();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "~" {
        return raw.to_string();
    }
    let name = file_name_of(trimmed);
    if name.is_empty() {
        return format!("{FOLDER_MARKER}{trail}");
    }
    if has_extension(&name) {
        format!("{name}{trail}")
    } else {
        format!("{FOLDER_MARKER}{trail}")
    }
}

fn scrub_text_plain(text: &str) -> String {
    let patterns = path_patterns();
    // `file://` first: its path part would otherwise match the Unix rule.
    let step = patterns
        .file_url
        .replace_all(text, |caps: &regex::Captures<'_>| scrub_match(&caps[0]))
        .into_owned();
    let step = patterns
        .windows
        .replace_all(&step, |caps: &regex::Captures<'_>| scrub_match(&caps[0]))
        .into_owned();
    let step = patterns
        .unc
        .replace_all(&step, |caps: &regex::Captures<'_>| scrub_match(&caps[0]))
        .into_owned();
    patterns
        .unix
        .replace_all(&step, |caps: &regex::Captures<'_>| {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let path = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            // A lone slash between words (`a / b`) is not a path.
            if path == "/" || path.is_empty() {
                return caps[0].to_string();
            }
            format!("{prefix}{}", scrub_match(path))
        })
        .into_owned()
}

fn scrub_string_for_key(key: &str, value: &str) -> String {
    if is_excluded_key(key) {
        return value.to_string();
    }
    if is_path_key(key) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return value.to_string();
        }
        // A path key with plain prose and no separator is not a path:
        // leave it, so a status word never becomes `[carpeta]`.
        if !looks_like_absolute_path(trimmed) && !trimmed.contains('/') && !trimmed.contains('\\') {
            return value.to_string();
        }
        return scrub_one_path_value(key, value);
    }
    // Any other field: only absolute-path substrings are replaced, so
    // `google/gemma` and ordinary prose travel untouched.
    scrub_text_plain(value)
}

fn scrub_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                match entry {
                    serde_json::Value::String(text) => {
                        *text = scrub_string_for_key(key, text);
                    }
                    other => scrub_json_value(other),
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                match item {
                    serde_json::Value::String(text) => {
                        *text = scrub_text_plain(text);
                    }
                    other => scrub_json_value(other),
                }
            }
        }
        _ => {}
    }
}

/// The text the model sees for one tool result: JSON payloads are walked
/// key by key (D18), any other text gets the same file-or-folder rule for
/// every absolute-path substring. IDs, titles, names and relative slugs
/// like `google/gemma` are left verbatim.
pub(crate) fn scrub_tool_content(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(content) {
            scrub_json_value(&mut value);
            return serde_json::to_string_pretty(&value).unwrap_or_else(|_| content.to_string());
        }
    }
    scrub_text_plain(content)
}

fn scrub_outcome(outcome: ToolOutcome) -> ToolOutcome {
    match outcome {
        ToolOutcome::Ok { content } => ToolOutcome::Ok {
            content: scrub_tool_content(&content),
        },
        ToolOutcome::Pending {
            content,
            approval_id,
            target,
        } => ToolOutcome::Pending {
            content: scrub_tool_content(&content),
            approval_id,
            target,
        },
        ToolOutcome::Denied { reason } => ToolOutcome::Denied {
            reason: scrub_tool_content(&reason),
        },
        ToolOutcome::Failed { message } => ToolOutcome::Failed {
            message: scrub_tool_content(&message),
        },
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
        app_handle: Option<tauri::AppHandle>,
    ) -> Result<Self, String> {
        let service = InProcessHelpServer {
            inner: AgentServer::new(
                db_path,
                cache_dir,
                approvals,
                corpus_events,
                access,
                app_handle,
            ),
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
    /// Every text is scrubbed (D18) before the model sees it.
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
            Err(error) => scrub_outcome(ToolOutcome::Failed {
                message: format!("La herramienta falló: {error}"),
            }),
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

    /// The 23 tools D10 serves, in `tools/list` order (mirrors the HTTP
    /// endpoint's list in `automation::server::tests`; `models_setup` came
    /// from main, 42c1bd4f).
    const EXPECTED_TOOLS: [&str; 23] = [
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
        "models_setup",
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
        let bridge =
            HelpMcpBridge::start(db_path, cache_dir, approvals, corpus_events, access, None)
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
            None,
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

    // -- D18: file-system paths never reach the model (T32) ------------------

    #[test]
    fn d18_items_get_asset_path_keeps_only_the_file_name() {
        // An `items_get`-shaped payload: the asset path loses its folders,
        // the id and the title travel untouched.
        let payload = serde_json::json!({
            "id": "ite_1",
            "title": "Acta del cabildo",
            "collectionId": "col_1",
            "assets": [{"id": "ast_1", "type": "pdf", "path": "C:\\Users\\ana\\Archivo\\Actas 1890\\acta.pdf"}],
        })
        .to_string();
        let scrubbed = super::scrub_tool_content(&payload);
        assert!(scrubbed.contains("acta.pdf"), "{scrubbed}");
        for leaked in ["ana", "Archivo", "Actas 1890", "C:\\Users"] {
            assert!(
                !scrubbed.contains(leaked),
                "{scrubbed} still has {leaked:?}"
            );
        }
        assert!(scrubbed.contains("ite_1"), "{scrubbed}");
        assert!(scrubbed.contains("Acta del cabildo"), "{scrubbed}");
    }

    #[test]
    fn d18_jobs_get_asset_path_unix_keeps_only_the_file_name() {
        let payload = serde_json::json!({
            "job": {"id": "job_1"},
            "tasks": [{"document": {"asset_path": "/home/ana/archivo/acta-12.pdf"}}],
        })
        .to_string();
        let scrubbed = super::scrub_tool_content(&payload);
        assert!(scrubbed.contains("acta-12.pdf"), "{scrubbed}");
        assert!(!scrubbed.contains("/home/ana"), "{scrubbed}");
        assert!(!scrubbed.contains("archivo"), "{scrubbed}");
        assert!(scrubbed.contains("job_1"), "{scrubbed}");
    }

    #[test]
    fn d18_import_sources_folder_roots_become_a_folder_marker() {
        let payload = serde_json::json!({
            "folders": [{"path": "/home/ana/Entrada", "files": [{"name": "Acta.pdf"}], "truncated": false}],
        })
        .to_string();
        let scrubbed = super::scrub_tool_content(&payload);
        assert!(scrubbed.contains("[carpeta]"), "{scrubbed}");
        assert!(!scrubbed.contains("/home/ana"), "{scrubbed}");
        assert!(!scrubbed.contains("Entrada"), "{scrubbed}");
        assert!(scrubbed.contains("Acta.pdf"), "file names stay: {scrubbed}");
    }

    #[test]
    fn d18_ids_titles_and_plain_slugs_stay_untouched() {
        let payload = serde_json::json!({
            "id": "ite_1",
            "itemId": "ite_1",
            "title": "Acta del cabildo",
            "model": "google/gemma",
        })
        .to_string();
        let scrubbed = super::scrub_tool_content(&payload);
        assert!(scrubbed.contains("ite_1"), "{scrubbed}");
        assert!(scrubbed.contains("Acta del cabildo"), "{scrubbed}");
        assert!(
            scrubbed.contains("google/gemma"),
            "a model slug is not a path: {scrubbed}"
        );
    }

    #[test]
    fn d18_plain_text_paths_use_the_same_file_or_folder_rule() {
        let scrubbed = super::scrub_tool_content("falló C:\\Users\\ana\\tesis.pdf con 401");
        assert!(scrubbed.contains("tesis.pdf"), "{scrubbed}");
        assert!(!scrubbed.contains("ana"), "{scrubbed}");
        let folder = super::scrub_tool_content("carpeta C:\\Users\\ana\\Entrada sin extensión");
        assert!(folder.contains("[carpeta]"), "{folder}");
        assert!(!folder.contains("ana"), "{folder}");
    }
}
