//! Help agent gate (T22, S15/S12, D12): which tools the model may use,
//! when document text may travel, and whose approval a change needs.
//!
//! Tool classes come from the bridge's tool names: READ runs freely,
//! CONTENT (`documents_get_text`, `search_passages`) is offered only with
//! «Buscar en mis documentos» ticked, LOCAL_WRITE asks the owner inline in
//! the help panel, and MCP_APPROVAL goes through as is (the MCP creates its
//! own approval, shown by `AgentApprovalDialog`). Unknown future tools
//! default to the most restrictive class that needs approval (fail closed).

use std::sync::{Arc, Mutex};

use super::agent_llm::{HelpToolExecutor, ToolOutcome, ToolSpec};
use super::manual_tool::{is_manual_tool, manual_tool_spec, ManualSearch};
use super::web_tools::{is_web_tool, HttpFetch, ReqwestFetch, WebTools};

/// What a tool may do, and whose approval a call needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolClass {
    /// Listings and metadata: run freely.
    Read,
    /// Returns the user's document text: only with the documents box ticked.
    Content,
    /// Changes data with no MCP approval of its own: inline «Aprobar /
    /// Rechazar» card in the help panel before forwarding.
    LocalWrite,
    /// Opens its own MCP approval (existing dialog): forward as is.
    McpApproval,
}

/// READ tools: listings and metadata. `search_items` is READ on purpose: it
/// returns `id`/`title`/`collectionId`/`rank` only (see
/// `automation::read::SearchRecord`), never text snippets, so it leaks no
/// document content with the box off.
const READ_TOOLS: &[&str] = &[
    "entropia_status",
    "approvals_status",
    "collections_list",
    "import_sources_list",
    "items_get",
    "items_list",
    "jobs_get",
    "jobs_list",
    "search_items",
    // The read-only web tools (T23, S16, D14): public repo and HLab page,
    // no approval, with or without the documents box.
    "repo_tree",
    "repo_read",
    "hlab_entropia_page",
    // The on-demand manual reader (S22, D19): answered from the local
    // index, no approval, with or without the documents box.
    "manual_search",
];

/// CONTENT tools: they return the user's document text (`documents_get_text`
/// pages, `search_passages` fragments), so the gate only offers them with
/// the documents box ticked (S12).
const CONTENT_TOOLS: &[&str] = &["documents_get_text", "search_passages"];

/// Local writes with no MCP approval of their own: the inline card asks
/// before anything is forwarded.
const LOCAL_WRITE_TOOLS: &[&str] = &[
    "collections_create",
    "collections_update",
    "items_update",
    "documents_index",
    "processing_prepare",
];

/// Changes that open their own MCP approval: forwarded as is, and the tool
/// result tells the model the request is pending.
const MCP_APPROVAL_TOOLS: &[&str] = &[
    "collections_delete",
    "documents_text_update",
    "documents_import",
    "documents_delete",
    "processing_start",
];

/// Which class a call belongs to. `processing_control` splits on its
/// `action`: pause/cancel apply at once (inline approval), resume/retry
/// spend again (MCP approval). A tool the gate does not know but the bridge
/// offers — a future tool — defaults to `LocalWrite`: the owner sees the
/// inline card before anything runs (fail closed). A name the bridge does
/// not offer at all is not classified here; `HelpGateExecutor::call`
/// denies it without asking.
pub(crate) fn classify_call(name: &str, args: &serde_json::Value) -> ToolClass {
    if READ_TOOLS.contains(&name) {
        return ToolClass::Read;
    }
    if CONTENT_TOOLS.contains(&name) {
        return ToolClass::Content;
    }
    if LOCAL_WRITE_TOOLS.contains(&name) {
        return ToolClass::LocalWrite;
    }
    if MCP_APPROVAL_TOOLS.contains(&name) {
        return ToolClass::McpApproval;
    }
    if name == "models_setup" {
        // `status` only reads the installed models; `install` starts
        // downloads of several GB, so the owner approves it first. Anything
        // else asks first: fail closed.
        return match args.get("action").and_then(|action| action.as_str()) {
            Some("status") => ToolClass::Read,
            _ => ToolClass::LocalWrite,
        };
    }
    if name == "engines_setup" {
        // `status` only reads the settings; `set` changes where work runs
        // and which providers receive data, so the owner approves it first.
        // Anything else asks first: fail closed.
        return match args.get("action").and_then(|action| action.as_str()) {
            Some("status") => ToolClass::Read,
            _ => ToolClass::LocalWrite,
        };
    }
    if name == "processing_control" {
        // pause/cancel apply at once (the gate asks inline); resume/retry
        // spend again (the MCP asks in its dialog). Anything else asks
        // inline: fail closed.
        return match args.get("action").and_then(|action| action.as_str()) {
            Some("pause") | Some("cancel") => ToolClass::LocalWrite,
            Some("resume") | Some("retry") => ToolClass::McpApproval,
            _ => ToolClass::LocalWrite,
        };
    }
    // A tool the gate does not know but the bridge offers — a future tool —
    // defaults to the most restrictive class that needs approval: the owner
    // sees the inline card before anything runs. Never run freely.
    ToolClass::LocalWrite
}

/// The one-line Spanish summary the inline card shows for a local write.
/// Known tools name what they will do; anything else falls back to the
/// tool name with its compact arguments.
pub(crate) fn approval_summary(tool: &str, args: &serde_json::Value) -> String {
    let field = |key: &str| {
        args.get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("")
    };
    let fallback = || {
        format!(
            "Usar {tool} con {args}",
            args = super::agent_llm::compact_args_preview(args)
        )
    };
    match tool {
        "collections_create" => match field("name") {
            "" => fallback(),
            name => format!("Crear la colección «{name}»"),
        },
        "collections_update" => match (field("name"), field("collectionId")) {
            ("", "") => fallback(),
            (name, "") => format!("Actualizar la colección «{name}»"),
            (_, id) => {
                let name = field("name");
                if name.is_empty() {
                    format!("Actualizar la colección «{id}»")
                } else {
                    format!("Actualizar la colección «{name}»")
                }
            }
        },
        "items_update" => match (field("title"), field("itemId")) {
            ("", "") => fallback(),
            (title, "") => format!("Actualizar el documento «{title}»"),
            (_, id) => {
                let title = field("title");
                if title.is_empty() {
                    format!("Actualizar el documento «{id}»")
                } else {
                    format!("Actualizar el documento «{title}»")
                }
            }
        },
        "documents_index" => match field("itemId") {
            "" => fallback(),
            id => format!("Reindexar «{id}» para la búsqueda"),
        },
        "processing_prepare" => {
            let operations = args
                .get("operations")
                .and_then(|value| value.as_array())
                .map(|operations| {
                    operations
                        .iter()
                        .filter_map(|operation| operation.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if operations.is_empty() {
                fallback()
            } else {
                format!("Preparar un procesamiento ({operations})")
            }
        }
        "models_setup" => {
            let components: Vec<&str> = args
                .get("components")
                .and_then(|value| value.as_array())
                .map(|list| list.iter().filter_map(|value| value.as_str()).collect())
                .unwrap_or_default();
            if components.is_empty() {
                "Instalar los modelos locales que faltan (puede bajar varios GB)".to_string()
            } else {
                format!(
                    "Instalar modelos locales: {} (puede bajar varios GB)",
                    components.join(", ")
                )
            }
        }
        "engines_setup" => {
            let mut changes: Vec<String> = Vec::new();
            if let Some(engines) = args.get("engines").and_then(|value| value.as_object()) {
                for (setting, value) in engines {
                    changes.push(format!("{setting} = {}", value.as_str().unwrap_or("?")));
                }
            }
            if let Some(apis) = args.get("apis").and_then(|value| value.as_object()) {
                for (api, on) in apis {
                    let state = if on.as_bool() == Some(true) {
                        "encender"
                    } else {
                        "apagar"
                    };
                    changes.push(format!("{state} la API {api}"));
                }
            }
            if changes.is_empty() {
                fallback()
            } else {
                format!("Cambiar motores y APIs: {}", changes.join(", "))
            }
        }
        "processing_control" => {
            let job = field("jobId");
            match (args.get("action").and_then(|action| action.as_str()), job) {
                (Some("pause"), job) if !job.is_empty() => {
                    format!("Pausar el procesamiento «{job}»")
                }
                (Some("cancel"), job) if !job.is_empty() => {
                    format!("Cancelar el procesamiento «{job}»")
                }
                _ => fallback(),
            }
        }
        _ => fallback(),
    }
}

/// The titles of the user's documents a CONTENT result carried, for
/// `HelpAnswer.documents`: `documents_get_text` reports `item.title`,
/// `search_passages` reports one `title` per record. Lenient on purpose:
/// anything unparsable yields no titles, never an error.
pub(crate) fn collect_content_titles(tool: &str, content: &str) -> Vec<String> {
    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let mut titles = Vec::new();
    let mut push = |title: Option<&str>| {
        let title = title.map(str::trim).unwrap_or("");
        if !title.is_empty() && !titles.contains(&title.to_string()) {
            titles.push(title.to_string());
        }
    };
    match tool {
        "documents_get_text" => {
            push(
                value
                    .pointer("/item/title")
                    .and_then(|title| title.as_str()),
            );
        }
        "search_passages" => {
            if let Some(records) = value.get("records").and_then(|records| records.as_array()) {
                for record in records {
                    push(record.get("title").and_then(|title| title.as_str()));
                }
            }
        }
        _ => {}
    }
    titles
}

/// The owner's answer to one inline approval card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalAnswer {
    Approved,
    Rejected,
    TimedOut,
}

/// Whoever asks the owner about a local write. Unit tests inject a scripted
/// approver; production implements it over a Tauri event plus a oneshot
/// (see `commands.rs`), so no Tauri type crosses this module.
pub(crate) trait HelpApprover: Send + Sync {
    fn request_approval(
        &self,
        tool: &str,
        args: serde_json::Value,
        summary: String,
    ) -> impl std::future::Future<Output = ApprovalAnswer> + Send;
}

/// The gate around a bridge executor: `tools()` hides CONTENT without the
/// documents box, `call()` enforces the class of every call and records
/// the document titles CONTENT results carried. An MCP approval (T26)
/// returns `Pending` with the approval id and the human target, and is
/// recorded for the background outcome watcher. The read-only web tools
/// travel inside the gate (T23, S16, D14): they run freely like READ.
pub(crate) struct HelpGateExecutor<E, A, H = ReqwestFetch> {
    inner: E,
    approver: A,
    with_documents: bool,
    content_titles: Arc<Mutex<Vec<String>>>,
    pending: Arc<Mutex<Vec<super::outcome::PendingApproval>>>,
    web: WebTools<H>,
    manual: ManualSearch,
}

impl<E, A> HelpGateExecutor<E, A, ReqwestFetch> {
    pub(crate) fn new(inner: E, approver: A, with_documents: bool, manual: ManualSearch) -> Self {
        Self {
            inner,
            approver,
            with_documents,
            content_titles: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(Mutex::new(Vec::new())),
            web: WebTools::new(ReqwestFetch::default()),
            manual,
        }
    }
}

impl<E, A, H: HttpFetch> HelpGateExecutor<E, A, H> {
    /// The same gate over an injected web layer: tests script the HTTP
    /// answers instead of touching the net.
    #[cfg(test)]
    pub(crate) fn new_with_web(
        inner: E,
        approver: A,
        with_documents: bool,
        web: WebTools<H>,
        manual: ManualSearch,
    ) -> Self {
        Self {
            inner,
            approver,
            with_documents,
            content_titles: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(Mutex::new(Vec::new())),
            web,
            manual,
        }
    }

    /// Titles CONTENT calls returned so far, in first-seen order.
    pub(crate) fn content_titles(&self) -> Vec<String> {
        self.content_titles
            .lock()
            .map(|titles| titles.clone())
            .unwrap_or_default()
    }

    /// Every manual/screen-map section `manual_search` returned so far,
    /// deduplicated, in first-seen order (S22, D19): the run's
    /// `HelpAnswer.sources` come from here and nowhere else.
    pub(crate) fn consulted_manual_sections(&self) -> Vec<super::index::Hit> {
        self.manual.consulted_sections()
    }
    /// Pending MCP approvals (T26) recorded so far, in call order: the
    /// id parsed from each pending tool result plus the human target.
    pub(crate) fn pending_approvals(&self) -> Vec<super::outcome::PendingApproval> {
        self.pending
            .lock()
            .map(|pending| pending.clone())
            .unwrap_or_default()
    }
}

impl<E, A, H> HelpToolExecutor for HelpGateExecutor<E, A, H>
where
    E: HelpToolExecutor,
    A: HelpApprover,
    H: HttpFetch,
{
    fn tools(&self) -> Vec<ToolSpec> {
        let all = self.inner.tools();
        let mut out: Vec<ToolSpec> = if self.with_documents {
            all
        } else {
            all.into_iter()
                .filter(|tool| {
                    classify_call(&tool.name, &serde_json::Value::Null) != ToolClass::Content
                })
                .collect()
        };
        // The read-only web tools (T23, D14): always offered, with or
        // without the documents box, and never gated by it. So is the
        // on-demand manual reader (S22, D19).
        out.extend(super::web_tools::tool_specs());
        out.extend([manual_tool_spec()]);
        out
    }

    async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
        // The web tools run freely (READ): public repo and HLab page, so
        // neither the documents box nor the owner is asked (T23, D14).
        if is_web_tool(name) {
            return self.web.call(name, args).await;
        }
        // The manual reader (S22, D19): the local index answers, so
        // neither the bridge nor the owner is asked, and what it returns
        // is recorded for `HelpAnswer.sources`.
        if is_manual_tool(name) {
            return self.manual.call(args).await;
        }
        if !self.inner.tools().iter().any(|tool| tool.name == name) {
            return ToolOutcome::Denied {
                reason: format!("herramienta desconocida '{name}'"),
            };
        }
        match classify_call(name, &args) {
            ToolClass::Read => self.inner.call(name, args).await,
            ToolClass::Content if !self.with_documents => ToolOutcome::Denied {
                reason: "El contenido de tus documentos solo se lee con «Buscar en mis documentos» activado.".to_string(),
            },
            ToolClass::Content => {
                let outcome = self.inner.call(name, args).await;
                if let ToolOutcome::Ok { content } = &outcome {
                    if let Ok(mut titles) = self.content_titles.lock() {
                        for title in collect_content_titles(name, content) {
                            if !titles.contains(&title) {
                                titles.push(title);
                            }
                        }
                    }
                }
                outcome
            }
            ToolClass::LocalWrite => {
                let summary = approval_summary(name, &args);
                match self.approver.request_approval(name, args.clone(), summary).await {
                    ApprovalAnswer::Approved => self.inner.call(name, args).await,
                    ApprovalAnswer::Rejected => ToolOutcome::Denied {
                        reason: "El usuario rechazó la acción.".to_string(),
                    },
                    ApprovalAnswer::TimedOut => ToolOutcome::Denied {
                        reason: "Sin respuesta en 10 minutos; no se ejecutó la acción.".to_string(),
                    },
                }
            }
            ToolClass::McpApproval => {
                // An MCP change never executes by itself (T26): the bridge
                // answers with a pending approval for the owner. The id
                // follows the outcome; the target names the step and the
                // outcome message (never a UUID). Unknown names stay
                // unknown: the generic fallback wins.
                match self.inner.call(name, args.clone()).await {
                    ToolOutcome::Ok { content } => {
                        match super::outcome::parse_pending_approval_id(&content) {
                            Some(approval_id) => {
                                let approval: serde_json::Value =
                                    serde_json::from_str(&content).unwrap_or_default();
                                let target = super::outcome::resolve_target(
                                    &self.inner,
                                    name,
                                    &args,
                                    &approval,
                                )
                                .await;
                                if let Ok(mut pending) = self.pending.lock() {
                                    pending.push(super::outcome::PendingApproval {
                                        approval_id: approval_id.clone(),
                                        tool: name.to_string(),
                                        args: args.clone(),
                                        target: target.clone(),
                                        request_id: None,
                                        conversation_id: None,
                                    });
                                }
                                ToolOutcome::Pending {
                                    content,
                                    approval_id,
                                    target,
                                }
                            }
                            None => ToolOutcome::Ok { content },
                        }
                    }
                    other => other,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::agent_llm::ToolSpec;
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct MockBridge {
        tools: Vec<String>,
        calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
        handler: Arc<Mutex<VecDeque<ToolOutcome>>>,
    }

    impl Clone for MockBridge {
        fn clone(&self) -> Self {
            Self {
                tools: self.tools.clone(),
                calls: Arc::clone(&self.calls),
                handler: Arc::clone(&self.handler),
            }
        }
    }

    impl MockBridge {
        fn new(tools: &[&str]) -> Self {
            Self {
                tools: tools.iter().map(|name| name.to_string()).collect(),
                calls: Arc::new(Mutex::new(Vec::new())),
                handler: Arc::new(Mutex::new(VecDeque::new())),
            }
        }

        fn calls(&self) -> Vec<(String, serde_json::Value)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl HelpToolExecutor for MockBridge {
        fn tools(&self) -> Vec<ToolSpec> {
            self.tools
                .iter()
                .map(|name| ToolSpec {
                    name: name.clone(),
                    description: format!("hace {name}"),
                    input_schema: serde_json::json!({"type": "object"}),
                })
                .collect()
        }

        async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
            self.calls
                .lock()
                .expect("lock")
                .push((name.to_string(), args));
            self.handler
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or(ToolOutcome::Ok {
                    content: format!("eco de {name}"),
                })
        }
    }

    struct MockApprover {
        answers: Arc<Mutex<VecDeque<ApprovalAnswer>>>,
        requests: Arc<Mutex<Vec<(String, serde_json::Value, String)>>>,
    }

    impl Clone for MockApprover {
        fn clone(&self) -> Self {
            Self {
                answers: Arc::clone(&self.answers),
                requests: Arc::clone(&self.requests),
            }
        }
    }

    impl MockApprover {
        fn new(answers: Vec<ApprovalAnswer>) -> Self {
            Self {
                answers: Arc::new(Mutex::new(answers.into())),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<(String, serde_json::Value, String)> {
            self.requests.lock().expect("lock").clone()
        }
    }

    impl HelpApprover for MockApprover {
        async fn request_approval(
            &self,
            tool: &str,
            args: serde_json::Value,
            summary: String,
        ) -> ApprovalAnswer {
            self.requests
                .lock()
                .expect("lock")
                .push((tool.to_string(), args, summary));
            self.answers
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or(ApprovalAnswer::Rejected)
        }
    }

    const ALL_TOOLS: &[&str] = &[
        "entropia_status",
        "approvals_status",
        "collections_list",
        "collections_create",
        "collections_update",
        "collections_delete",
        "import_sources_list",
        "items_get",
        "items_list",
        "items_update",
        "jobs_get",
        "jobs_list",
        "documents_get_text",
        "documents_text_update",
        "documents_import",
        "documents_delete",
        "documents_index",
        "search_items",
        "search_passages",
        "processing_prepare",
        "processing_start",
        "processing_control",
        "models_setup",
        "engines_setup",
    ];

    // -- Behaviour through the executor -------------------------------------

    /// One executor plus live handles to both mocks (clones share the state).
    fn harness(
        with_documents: bool,
        answers: Vec<ApprovalAnswer>,
    ) -> (
        HelpGateExecutor<MockBridge, MockApprover>,
        MockBridge,
        MockApprover,
    ) {
        let bridge = MockBridge::new(ALL_TOOLS);
        let approver = MockApprover::new(answers);
        let executor =
            HelpGateExecutor::new(bridge.clone(), approver.clone(), with_documents, manual());
        (executor, bridge, approver)
    }

    /// One searcher over a tiny in-memory index for gate tests: no temp
    /// dir, no files, the same dispatch code as production.
    fn manual() -> super::super::manual_tool::ManualSearch {
        super::super::manual_tool::memory_searcher()
    }

    #[test]
    fn manual_search_is_read_and_always_offered() {
        // S22/D19: `manual_search` runs freely (READ class in the gate,
        // no approval) and is always offered, with or without the
        // documents box.
        for with_documents in [false, true] {
            let (executor, _, _) = harness(with_documents, vec![]);
            let names: Vec<String> = executor
                .tools()
                .iter()
                .map(|tool| tool.name.clone())
                .collect();
            assert!(
                names.contains(&"manual_search".to_string()),
                "con documentos={with_documents}: falta manual_search"
            );
        }
        assert_eq!(
            classify_call("manual_search", &serde_json::Value::Null),
            ToolClass::Read,
            "manual_search lee el índice local: corre sin aprobación"
        );
    }

    #[tokio::test]
    async fn manual_search_runs_without_approval_and_records_sections() {
        // S22/D19: the gate answers from the local index — the bridge is
        // never touched, the owner is never asked — and records what it
        // returned for `HelpAnswer.sources`.
        let (executor, bridge, approver) = harness(false, vec![]);
        match executor
            .call(
                "manual_search",
                serde_json::json!({"query": "importar archivos pdf"}),
            )
            .await
        {
            ToolOutcome::Ok { content } => assert!(
                content.contains("1.1. Importar archivos"),
                "the excerpt travels headed: {content}"
            ),
            other => panic!("manual_search must run: {other:?}"),
        }
        assert!(
            bridge.calls().is_empty(),
            "the index answers, never the bridge"
        );
        assert!(approver.requests().is_empty(), "leer nunca pregunta");
        let consulted = executor.consulted_manual_sections();
        assert!(!consulted.is_empty(), "what it returned is recorded");
        assert!(
            consulted
                .iter()
                .all(|hit| hit.source == super::super::index::Source::Manual
                    || hit.source == super::super::index::Source::ScreenMap),
            "kinds as today: {consulted:?}"
        );
    }

    #[test]
    fn tools_without_the_box_hide_content_but_keep_search_items() {
        let (executor, _, _) = harness(false, vec![]);
        let names: Vec<String> = executor
            .tools()
            .iter()
            .map(|tool| tool.name.clone())
            .collect();
        assert!(!names.contains(&"documents_get_text".to_string()));
        assert!(!names.contains(&"search_passages".to_string()));
        assert!(names.contains(&"search_items".to_string()));
        assert!(names.contains(&"collections_list".to_string()));
    }

    #[test]
    fn tools_with_the_box_offer_everything() {
        let (executor, _, _) = harness(true, vec![]);
        let names: Vec<String> = executor
            .tools()
            .iter()
            .map(|tool| tool.name.clone())
            .collect();
        assert!(names.contains(&"documents_get_text".to_string()));
        assert!(names.contains(&"search_passages".to_string()));
        assert_eq!(
            names.len(),
            ALL_TOOLS.len() + 4,
            "más las tres web y manual_search"
        );
    }

    #[tokio::test]
    async fn content_without_the_box_is_denied_and_never_forwarded() {
        let (executor, bridge, approver) = harness(false, vec![]);
        match executor
            .call("documents_get_text", serde_json::json!({"itemId": "ite_1"}))
            .await
        {
            ToolOutcome::Denied { reason } => assert!(!reason.trim().is_empty()),
            other => panic!("content without the box must be Denied: {other:?}"),
        }
        assert!(bridge.calls().is_empty(), "never forwarded");
        assert!(approver.requests().is_empty(), "never asked");
    }

    #[tokio::test]
    async fn content_with_the_box_is_forwarded_and_titles_recorded() {
        let (executor, bridge, _) = harness(true, vec![]);
        let text = serde_json::json!({
            "item": {"id": "ite_1", "title": "Acta del cabildo"},
            "pages": [],
            "nextCursor": null,
        })
        .to_string();
        // The bridge answers with the text payload; the gate records titles.
        let bridge2 = bridge.clone();
        bridge2
            .handler
            .lock()
            .expect("lock")
            .push_back(ToolOutcome::Ok { content: text });
        match executor
            .call("documents_get_text", serde_json::json!({"itemId": "ite_1"}))
            .await
        {
            ToolOutcome::Ok { .. } => {}
            other => panic!("content with the box must run: {other:?}"),
        }
        assert_eq!(bridge.calls().len(), 1);
        assert_eq!(
            executor.content_titles(),
            vec!["Acta del cabildo".to_string()]
        );
    }

    #[tokio::test]
    async fn approved_local_write_is_forwarded() {
        let (executor, bridge, approver) = harness(false, vec![ApprovalAnswer::Approved]);
        match executor
            .call(
                "collections_create",
                serde_json::json!({"requestId": "r-1", "name": "SIMAPE"}),
            )
            .await
        {
            ToolOutcome::Ok { .. } => {}
            other => panic!("an approved write must run: {other:?}"),
        }
        assert_eq!(bridge.calls().len(), 1);
        let requests = approver.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "collections_create");
        assert_eq!(requests[0].2, "Crear la colección «SIMAPE»");
    }

    #[tokio::test]
    async fn rejected_local_write_is_denied_and_never_forwarded() {
        let (executor, bridge, approver) = harness(false, vec![ApprovalAnswer::Rejected]);
        match executor
            .call(
                "collections_create",
                serde_json::json!({"requestId": "r-1", "name": "SIMAPE"}),
            )
            .await
        {
            ToolOutcome::Denied { reason } => assert!(
                reason.contains("rechazó"),
                "the denial says the user refused: {reason}"
            ),
            other => panic!("a rejected write must be Denied: {other:?}"),
        }
        assert!(bridge.calls().is_empty(), "never forwarded");
        assert_eq!(approver.requests().len(), 1);
    }

    #[tokio::test]
    async fn unanswered_local_write_times_out_denied() {
        let (executor, bridge, _) = harness(false, vec![ApprovalAnswer::TimedOut]);
        match executor
            .call(
                "processing_control",
                serde_json::json!({"action": "pause", "jobId": "j", "revision": 1}),
            )
            .await
        {
            ToolOutcome::Denied { reason } => assert!(
                reason.contains("10 minutos"),
                "the denial names the wait: {reason}"
            ),
            other => panic!("an unanswered write must be Denied: {other:?}"),
        }
        assert!(bridge.calls().is_empty(), "never forwarded");
    }

    #[tokio::test]
    async fn mcp_approval_calls_forward_as_pending_with_name() {
        // T26: an MCP change is forwarded, but the gate marks it pending
        // with the approval id and the collection NAME (never the UUID).
        let (executor, bridge, approver) = harness(false, vec![]);
        let pending = serde_json::json!({
            "status": "pending",
            "id": "apr_1",
            "clientId": "in-app-help",
            "clientName": "Ayuda",
            "collections": ["SIMAPE"],
            "files": 2,
            "documentTitles": ["Acta"],
        })
        .to_string();
        bridge
            .handler
            .lock()
            .expect("lock")
            .push_back(ToolOutcome::Ok {
                content: pending.clone(),
            });
        match executor
            .call(
                "collections_delete",
                serde_json::json!({"collectionId": "29189757-1545-411c-b1cc-93132f7aaab7"}),
            )
            .await
        {
            ToolOutcome::Pending {
                content,
                approval_id,
                target,
            } => {
                assert_eq!(content, pending);
                assert_eq!(approval_id, "apr_1");
                assert_eq!(target, Some("SIMAPE".to_string()));
            }
            other => panic!("an MCP approval call must be Pending: {other:?}"),
        }
        assert_eq!(
            bridge.calls().len(),
            1,
            "no extra reads: the name came with the pending"
        );
        assert!(
            approver.requests().is_empty(),
            "no inline card: the MCP dialog asks"
        );
        let recorded = executor.pending_approvals();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].approval_id, "apr_1");
        assert_eq!(recorded[0].target, Some("SIMAPE".to_string()));
    }

    #[tokio::test]
    async fn mcp_approval_without_a_name_falls_back_to_a_bridge_read() {
        // T26: when the pending payload names nothing, one cheap READ
        // resolves the name; a step never shows the raw UUID.
        let (executor, bridge, _) = harness(false, vec![]);
        let pending = serde_json::json!({
            "status": "pending",
            "id": "apr_9",
            "clientId": "in-app-help",
            "clientName": "Ayuda",
            "collections": [],
        })
        .to_string();
        let page = serde_json::json!({
            "records": [{"id": "col_1", "name": "SIMAPE", "description": null, "itemCount": 1, "createdAt": 1, "updatedAt": 1}],
            "nextCursor": null,
        })
        .to_string();
        {
            let mut handler = bridge.handler.lock().expect("lock");
            handler.push_back(ToolOutcome::Ok { content: pending });
            handler.push_back(ToolOutcome::Ok { content: page });
        }
        match executor
            .call(
                "collections_delete",
                serde_json::json!({"collectionId": "col_1"}),
            )
            .await
        {
            ToolOutcome::Pending { target, .. } => {
                assert_eq!(target, Some("SIMAPE".to_string()));
            }
            other => panic!("an MCP approval call must be Pending: {other:?}"),
        }
    }

    #[tokio::test]
    async fn hallucinated_tool_is_denied_without_asking() {
        let (executor, bridge, approver) = harness(true, vec![ApprovalAnswer::Approved]);
        match executor.call("no_existe", serde_json::json!({})).await {
            ToolOutcome::Denied { reason } => assert!(reason.contains("desconocida")),
            other => panic!("a hallucinated tool must be Denied: {other:?}"),
        }
        assert!(bridge.calls().is_empty(), "never forwarded");
        assert!(approver.requests().is_empty(), "never asked");
    }

    #[tokio::test]
    async fn unknown_but_offered_tool_asks_inline_before_running() {
        // A future tool the bridge offers and the gate does not classify:
        // fail closed behind the inline card.
        let tools: Vec<&str> = ALL_TOOLS
            .iter()
            .copied()
            .chain(["futura_herramienta"])
            .collect();
        let bridge = MockBridge::new(&tools);
        let approver = MockApprover::new(vec![ApprovalAnswer::Approved]);
        let executor = HelpGateExecutor::new(bridge.clone(), approver.clone(), false, manual());
        match executor
            .call("futura_herramienta", serde_json::json!({"a": 1}))
            .await
        {
            ToolOutcome::Ok { .. } => {}
            other => panic!("an approved future tool must run: {other:?}"),
        }
        let requests = approver.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].2, "Usar futura_herramienta con {\"a\":1}");
    }

    #[test]
    fn read_tools_run_freely() {
        for tool in [
            "entropia_status",
            "approvals_status",
            "collections_list",
            "import_sources_list",
            "items_get",
            "items_list",
            "jobs_get",
            "jobs_list",
        ] {
            assert_eq!(
                classify_call(tool, &serde_json::Value::Null),
                ToolClass::Read,
                "{tool} must run without approval"
            );
        }
    }

    #[test]
    fn search_items_is_read_because_it_returns_no_text() {
        // `automation::read::SearchRecord` carries id/title/collection/rank
        // only: metadata, no snippets. With the documents box off the model
        // may still search, and only open the text with the box on.
        assert_eq!(
            classify_call("search_items", &serde_json::json!({"query": "revista"})),
            ToolClass::Read
        );
    }

    #[test]
    fn content_tools_need_the_documents_box() {
        for tool in ["documents_get_text", "search_passages"] {
            assert_eq!(
                classify_call(tool, &serde_json::Value::Null),
                ToolClass::Content,
                "{tool} returns document text"
            );
        }
    }

    #[test]
    fn local_writes_need_inline_approval() {
        for tool in [
            "collections_create",
            "collections_update",
            "items_update",
            "documents_index",
            "processing_prepare",
        ] {
            assert_eq!(
                classify_call(tool, &serde_json::Value::Null),
                ToolClass::LocalWrite,
                "{tool} changes data with no MCP approval of its own"
            );
        }
        assert_eq!(
            classify_call(
                "processing_control",
                &serde_json::json!({"action": "pause", "jobId": "j", "revision": 1})
            ),
            ToolClass::LocalWrite,
            "pause applies at once"
        );
        assert_eq!(
            classify_call(
                "processing_control",
                &serde_json::json!({"action": "cancel", "jobId": "j", "revision": 1})
            ),
            ToolClass::LocalWrite,
            "cancel applies at once"
        );
    }

    #[test]
    fn mcp_approval_tools_forward_as_is() {
        for tool in [
            "collections_delete",
            "documents_text_update",
            "documents_import",
            "documents_delete",
            "processing_start",
        ] {
            assert_eq!(
                classify_call(tool, &serde_json::Value::Null),
                ToolClass::McpApproval,
                "{tool} opens its own MCP approval"
            );
        }
        assert_eq!(
            classify_call(
                "processing_control",
                &serde_json::json!({"action": "resume", "jobId": "j", "revision": 2})
            ),
            ToolClass::McpApproval,
            "resume spends again"
        );
        assert_eq!(
            classify_call(
                "processing_control",
                &serde_json::json!({"action": "retry", "jobId": "j", "revision": 2})
            ),
            ToolClass::McpApproval,
            "retry spends again"
        );
    }

    #[test]
    fn unknown_future_tools_fail_closed_behind_inline_approval() {
        assert_eq!(
            classify_call("futura_herramienta", &serde_json::json!({})),
            ToolClass::LocalWrite,
            "what the gate does not know must ask, never run freely"
        );
        assert_eq!(
            classify_call(
                "processing_control",
                &serde_json::json!({"action": "desconocida"})
            ),
            ToolClass::LocalWrite,
            "an unknown control action must ask, never run freely"
        );
    }

    #[test]
    fn models_setup_status_reads_freely_and_install_asks_first() {
        assert_eq!(
            classify_call("models_setup", &serde_json::json!({"action": "status"})),
            ToolClass::Read
        );
        assert_eq!(
            classify_call("models_setup", &serde_json::json!({"action": "install"})),
            ToolClass::LocalWrite
        );
        // Anything else (missing or unknown action) asks first: fail closed.
        assert_eq!(
            classify_call("models_setup", &serde_json::json!({})),
            ToolClass::LocalWrite
        );
        assert_eq!(
            approval_summary("models_setup", &serde_json::json!({"action": "install"})),
            "Instalar los modelos locales que faltan (puede bajar varios GB)"
        );
        assert_eq!(
            approval_summary(
                "models_setup",
                &serde_json::json!({"action": "install", "components": ["llm", "glm_ocr"]})
            ),
            "Instalar modelos locales: llm, glm_ocr (puede bajar varios GB)"
        );
    }

    #[test]
    fn engines_setup_status_reads_freely_and_set_asks_first() {
        assert_eq!(
            classify_call("engines_setup", &serde_json::json!({"action": "status"})),
            ToolClass::Read
        );
        assert_eq!(
            classify_call("engines_setup", &serde_json::json!({"action": "set"})),
            ToolClass::LocalWrite
        );
        assert_eq!(
            classify_call("engines_setup", &serde_json::json!({})),
            ToolClass::LocalWrite
        );
        assert_eq!(
            approval_summary(
                "engines_setup",
                &serde_json::json!({
                    "action": "set",
                    "engines": {"llm_mode": "openrouter"},
                    "apis": {"assemblyai": false}
                })
            ),
            "Cambiar motores y APIs: llm_mode = openrouter, apagar la API assemblyai"
        );
    }

    #[test]
    fn approval_summary_names_what_the_write_will_do() {
        assert_eq!(
            approval_summary(
                "collections_create",
                &serde_json::json!({"name": "SIMAPE", "requestId": "r-1"})
            ),
            "Crear la colección «SIMAPE»"
        );
    }

    #[test]
    fn approval_summary_falls_back_to_tool_and_args() {
        assert_eq!(
            approval_summary("futura_herramienta", &serde_json::json!({"a": 1})),
            "Usar futura_herramienta con {\"a\":1}"
        );
    }

    #[test]
    fn content_titles_come_from_the_text_and_passage_payloads() {
        let text = serde_json::json!({
            "item": {"id": "ite_1", "title": "Acta del cabildo"},
            "pages": [],
            "nextCursor": null,
        })
        .to_string();
        assert_eq!(
            collect_content_titles("documents_get_text", &text),
            vec!["Acta del cabildo".to_string()]
        );
        let passages = serde_json::json!({
            "records": [
                {"itemId": "a", "title": "Crónica del puerto"},
                {"itemId": "b", "title": "Crónica del puerto"},
                {"itemId": "c", "title": "Revista"},
            ],
        })
        .to_string();
        assert_eq!(
            collect_content_titles("search_passages", &passages),
            vec!["Crónica del puerto".to_string(), "Revista".to_string()],
            "one title per document, in first-seen order"
        );
        assert!(
            collect_content_titles("search_passages", "no es json").is_empty(),
            "unparsable content yields no titles, never an error"
        );
    }

    // -- Web tools (T23, S16, D14): the READ class --------------------------

    #[test]
    fn gate_offers_the_three_web_tools_with_and_without_the_box() {
        for with_documents in [false, true] {
            let (executor, _, _) = harness(with_documents, vec![]);
            let names: Vec<String> = executor
                .tools()
                .iter()
                .map(|tool| tool.name.clone())
                .collect();
            for tool in ["repo_tree", "repo_read", "hlab_entropia_page"] {
                assert!(
                    names.contains(&tool.to_string()),
                    "con documentos={with_documents}: falta {tool}"
                );
            }
        }
    }

    #[test]
    fn web_tools_classify_as_read() {
        for tool in ["repo_tree", "repo_read", "hlab_entropia_page"] {
            assert_eq!(
                classify_call(tool, &serde_json::Value::Null),
                ToolClass::Read,
                "{tool} es de solo lectura: corre sin aprobación"
            );
        }
    }

    #[tokio::test]
    async fn web_tools_run_without_asking() {
        use super::super::web_tools::test_support::{tree_body, MockFetch};
        use super::super::web_tools::{WebTools, GITHUB_API_TREE_URL};
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("README.md", Some(3))]),
        );
        let log = fetch.log();
        let web = WebTools::new(fetch);
        let bridge = MockBridge::new(ALL_TOOLS);
        let approver = MockApprover::new(vec![]);
        let executor =
            HelpGateExecutor::new_with_web(bridge.clone(), approver.clone(), false, web, manual());
        match executor.call("repo_tree", serde_json::json!({})).await {
            ToolOutcome::Ok { content } => assert!(content.contains("README.md")),
            other => panic!("repo_tree debe correr libre: {other:?}"),
        }
        assert!(bridge.calls().is_empty(), "la web no toca el puente");
        assert!(approver.requests().is_empty(), "leer nunca pregunta");
        assert_eq!(log.lock().expect("lock").len(), 1);
    }
}
