//! MCP approval outcomes for the in-app help assistant (T26, S15).
//!
//! An `MCP_APPROVAL` tool (`collections_delete`, `documents_delete`,
//! `documents_text_update`, `documents_import`, `processing_start`,
//! `processing_control` resume/retry) never executes by itself: the bridge
//! answers with a pending approval (`{status: "pending", id: "apr_..."}`)
//! and the owner decides in the existing MCP approval dialog. The help run
//! is usually over by then, so the outcome is followed in the background
//! and reported through `help://approval-outcome`:
//! `{ approvalId, status: approved|rejected|expired|failed, message }`.
//!
//! Names, never UUIDs: the pending approval JSON already carries the human
//! names the dialog shows (`collections`, `documentTitles`, `importFiles`,
//! file counts). [`extract_target`] reads those (plus the tool arguments);
//! when they are missing, [`resolve_target`] does one cheap READ through
//! the bridge (`collections_list` / `items_get`). A step whose name is
//! still unknown falls back to a generic («una colección»), never to a raw
//! id. No Tauri type crosses this module: the watcher polls through the
//! injectable [`ApprovalReader`] and reports through a plain `emit`
//! callback, so unit tests never touch Tauri.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::agent_llm::HelpToolExecutor;
use crate::automation::approvals::{Approval, ApprovalStatus};

/// The event the frontend listens to for MCP approval outcomes (T26):
/// one per decided help approval, `{ approvalId, status, message }`.
pub(crate) const HELP_APPROVAL_OUTCOME_EVENT: &str = "help://approval-outcome";

/// How long an outcome is followed after the answer (T26): the same
/// 10 minutes the approval itself lives (`APPROVAL_TTL_MS`).
pub(crate) const OUTCOME_WATCH_TIMEOUT: Duration = Duration::from_secs(600);

/// Between two outcome polls.
pub(crate) const OUTCOME_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The help assistant's client id, the only one the watcher reads.
pub(crate) const IN_APP_HELP_CLIENT_ID: &str = "in-app-help";

/// One help approval the background watcher follows: the id parsed from
/// the tool result plus what names the outcome message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingApproval {
    pub approval_id: String,
    pub tool: String,
    pub args: serde_json::Value,
    /// The human name (`SIMAPE`, `Acta del cabildo`), or `None` while
    /// unknown: messages and step labels fall back to a generic.
    pub target: Option<String>,
    /// The frontend question that requested it, when known.
    pub request_id: Option<String>,
    /// The stored help conversation it belongs to (S19, D17), when the
    /// question carried one: the outcome watcher appends its message
    /// there. `None` for runs without history.
    pub conversation_id: Option<String>,
}

/// One outcome over [`HELP_APPROVAL_OUTCOME_EVENT`] (T26).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutcomeEvent {
    pub approval_id: String,
    pub status: String,
    pub message: String,
}

/// The wire status of one decided approval.
pub(crate) fn outcome_status_wire(status: &ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Failed => "failed",
        ApprovalStatus::Rejected => "rejected",
        ApprovalStatus::Expired => "expired",
    }
}

/// The approval id of a pending MCP tool result, if it is one: the bridge
/// answers `{status: "pending", id: "apr_..."}` (plus names and counts).
/// Anything else — a refusal, a failure, a read payload — yields `None`.
pub(crate) fn parse_pending_approval_id(content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    if value.get("status")?.as_str()? != "pending" {
        return None;
    }
    let id = value.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

fn first_string(value: &serde_json::Value, pointer: &str) -> Option<String> {
    let text = value.pointer(pointer)?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// The human target of one pending approval from its own payload: the
/// tool arguments plus the approval JSON the bridge returned (the same
/// names the dialog shows). Never a UUID: `None` means "use the generic".
pub(crate) fn extract_target(
    tool: &str,
    args: &serde_json::Value,
    approval: &serde_json::Value,
) -> Option<String> {
    match tool {
        "collections_delete" | "documents_import" => {
            // `collections[0]` is the collection NAME the corpus layer
            // looked up (see `corpus_tools::request_*`).
            first_string(approval, "/collections/0")
        }
        "documents_delete" => {
            let titles = approval
                .pointer("/documentTitles")
                .and_then(|titles| titles.as_array());
            match titles.map(|titles| titles.len()).unwrap_or(0) {
                0 => None,
                1 => first_string(approval, "/documentTitles/0"),
                count => Some(format!("{count} documentos")),
            }
        }
        // `documents_text_update` carries no item title (only the
        // collection name): the bridge read in `resolve_target` fills it.
        // Processing approvals name operations, not one object: generic.
        _ => {
            let _ = args;
            None
        }
    }
}

/// One cheap READ through the bridge to name what the approval payload
/// did not: the collection of a delete/import, the item of a text update
/// or of a single-document delete. Anything unparsable yields `None`.
pub(crate) async fn resolve_target<E>(
    executor: &E,
    tool: &str,
    args: &serde_json::Value,
    approval: &serde_json::Value,
) -> Option<String>
where
    E: HelpToolExecutor,
{
    if let Some(target) = extract_target(tool, args, approval) {
        return Some(target);
    }
    match tool {
        "collections_delete" => {
            let id = args.get("collectionId")?.as_str()?;
            collection_name(executor, id).await
        }
        "documents_import" => {
            let id = args.get("collectionId")?.as_str()?;
            collection_name(executor, id).await
        }
        "documents_text_update" => {
            let id = args.get("itemId")?.as_str()?;
            item_title(executor, id).await
        }
        "documents_delete" => {
            let ids = args.get("itemIds")?.as_array()?;
            if ids.len() == 1 {
                let id = ids.first()?.as_str()?;
                item_title(executor, id).await
            } else {
                None
            }
        }
        _ => None,
    }
}

async fn collection_name<E>(executor: &E, id: &str) -> Option<String>
where
    E: HelpToolExecutor,
{
    let mut cursor: Option<String> = None;
    // At most three pages: a name is worth a short look, not a full scan.
    for _ in 0..3 {
        let mut call = serde_json::json!({"limit": 200});
        if let Some(cursor) = cursor.take() {
            call["cursor"] = serde_json::Value::String(cursor);
        }
        let content = match executor.call("collections_list", call).await {
            super::agent_llm::ToolOutcome::Ok { content } => content,
            _ => return None,
        };
        let page: serde_json::Value = serde_json::from_str(&content).ok()?;
        for record in page.get("records")?.as_array()? {
            if record.get("id")?.as_str()? == id {
                return first_string(record, "/name");
            }
        }
        cursor = page
            .get("nextCursor")
            .and_then(|cursor| cursor.as_str())
            .map(str::to_string);
        cursor.as_ref()?;
    }
    None
}

async fn item_title<E>(executor: &E, id: &str) -> Option<String>
where
    E: HelpToolExecutor,
{
    let content = match executor
        .call("items_get", serde_json::json!({"itemId": id}))
        .await
    {
        super::agent_llm::ToolOutcome::Ok { content } => content,
        _ => return None,
    };
    let detail: serde_json::Value = serde_json::from_str(&content).ok()?;
    first_string(&detail, "/title")
}

/// Whoever the watcher reads approvals through. Production polls the
/// shared [`crate::automation::approvals::ApprovalStore`] for the help
/// client; tests inject a scripted reader. No Tauri here.
pub(crate) trait ApprovalReader: Send + Sync {
    fn read(&self, approval_id: &str) -> Option<Approval>;
}

/// The shared store, scoped to the help assistant's own requests.
pub(crate) struct StoreApprovals {
    store: Arc<crate::automation::approvals::ApprovalStore>,
}

impl StoreApprovals {
    pub(crate) fn new(store: Arc<crate::automation::approvals::ApprovalStore>) -> Self {
        Self { store }
    }
}

impl ApprovalReader for StoreApprovals {
    fn read(&self, approval_id: &str) -> Option<Approval> {
        self.store
            .get_for_client(IN_APP_HELP_CLIENT_ID, approval_id)
    }
}

fn quote(name: &str) -> String {
    format!("«{name}»")
}

/// The clear Spanish outcome message (T26): what happened to the named
/// target, never a UUID. `approval` is the decided store row, so an
/// approved-and-executed import reports what it did and an
/// approved-but-failed one reports why (`failure`).
pub(crate) fn outcome_message(
    tool: &str,
    args: &serde_json::Value,
    target: Option<&str>,
    approval: &Approval,
) -> String {
    let failure = approval
        .failure
        .as_deref()
        .map(str::trim)
        .filter(|failure| !failure.is_empty())
        .unwrap_or("no se ejecutó nada");
    match approval.status {
        ApprovalStatus::Approved => approved_message(tool, args, target, approval),
        ApprovalStatus::Rejected => rejected_message(tool, args, target),
        ApprovalStatus::Expired => expired_message(tool, args, target),
        ApprovalStatus::Failed => {
            format!("{}: {failure}.", failed_prefix(tool, args, target))
        }
        ApprovalStatus::Pending => "Todavía está pendiente.".to_string(),
    }
}

fn collection_phrase(target: Option<&str>) -> String {
    match target {
        Some(name) => format!("la colección {}", quote(name)),
        None => "una colección".to_string(),
    }
}

fn approved_message(
    tool: &str,
    args: &serde_json::Value,
    target: Option<&str>,
    approval: &Approval,
) -> String {
    match tool {
        "collections_delete" => format!("Se eliminó {}.", collection_phrase(target)),
        "documents_delete" => {
            let count = approval.files.max(1);
            if count <= 1 {
                match target {
                    // A multi-document target already counts itself.
                    Some(name) if name.contains("documentos") => {
                        format!("Se eliminaron {name}.")
                    }
                    Some(title) => format!("Se eliminó el documento {}.", quote(title)),
                    None => "Se eliminó un documento.".to_string(),
                }
            } else {
                match target {
                    Some(name) if name.contains("documentos") => {
                        format!("Se eliminaron {name}.")
                    }
                    _ => format!("Se eliminaron {count} documentos."),
                }
            }
        }
        "documents_text_update" => match target {
            Some(title) => format!("Se actualizó el texto de {}.", quote(title)),
            None => "Se actualizó el texto de un documento.".to_string(),
        },
        "documents_import" => {
            let collection = match target {
                Some(name) => format!("la colección {}", quote(name)),
                None => "una colección".to_string(),
            };
            match &approval.import_result {
                Some(result) => {
                    let imported = result.imported.len();
                    let skipped = result.skipped.len();
                    if skipped == 0 {
                        if imported == 1 {
                            format!("Se importó 1 archivo a {collection}.")
                        } else {
                            format!("Se importaron {imported} archivos a {collection}.")
                        }
                    } else if imported == 1 {
                        format!("Se importó 1 archivo a {collection}; {skipped} se saltearon.")
                    } else {
                        format!(
                            "Se importaron {imported} archivos a {collection}; {skipped} se saltearon."
                        )
                    }
                }
                None => format!("Se aprobó importar archivos a {collection}."),
            }
        }
        "processing_start" => "Se inició el procesamiento.".to_string(),
        "processing_control" => match args.get("action").and_then(|action| action.as_str()) {
            Some("resume") => "Se reanudó el procesamiento.".to_string(),
            Some("retry") => "Se reintentó el procesamiento.".to_string(),
            _ => "Se aplicó el control del procesamiento.".to_string(),
        },
        _ => format!("Se aprobó «{tool}»."),
    }
}

fn rejected_message(tool: &str, args: &serde_json::Value, target: Option<&str>) -> String {
    match tool {
        "collections_delete" => format!(
            "Rechazaste borrar {}; no se borró nada.",
            collection_phrase(target)
        ),
        "documents_delete" => match target {
            Some(name) if name.contains("documentos") => {
                format!("Rechazaste borrar {name}; no se borró nada.")
            }
            Some(title) => format!(
                "Rechazaste borrar el documento {}; no se borró nada.",
                quote(title)
            ),
            None => "Rechazaste borrar un documento; no se borró nada.".to_string(),
        },
        "documents_text_update" => match target {
            Some(title) => format!(
                "Rechazaste actualizar el texto de {}; no cambió nada.",
                quote(title)
            ),
            None => "Rechazaste actualizar el texto de un documento; no cambió nada.".to_string(),
        },
        "documents_import" => match target {
            Some(name) => format!(
                "Rechazaste importar archivos a la colección {}; no se importó nada.",
                quote(name)
            ),
            None => "Rechazaste importar archivos; no se importó nada.".to_string(),
        },
        "processing_start" => "Rechazaste iniciar el procesamiento; no se inició nada.".to_string(),
        "processing_control" => match args.get("action").and_then(|action| action.as_str()) {
            Some("resume") => "Rechazaste reanudar el procesamiento; no se hizo nada.".to_string(),
            Some("retry") => "Rechazaste reintentar el procesamiento; no se hizo nada.".to_string(),
            _ => "Rechazaste el control del procesamiento; no se hizo nada.".to_string(),
        },
        _ => format!("Rechazaste «{tool}»; no se hizo nada."),
    }
}

fn expired_message(tool: &str, args: &serde_json::Value, target: Option<&str>) -> String {
    match tool {
        "collections_delete" => format!(
            "El pedido para borrar {} venció; no se hizo nada.",
            collection_phrase(target)
        ),
        "documents_delete" => {
            "El pedido para borrar documentos venció; no se hizo nada.".to_string()
        }
        "documents_text_update" => {
            "El pedido para actualizar el texto venció; no cambió nada.".to_string()
        }
        "documents_import" => {
            "El pedido para importar archivos venció; no se importó nada.".to_string()
        }
        "processing_start" => {
            "El pedido para iniciar el procesamiento venció; no se inició nada.".to_string()
        }
        "processing_control" => match args.get("action").and_then(|action| action.as_str()) {
            Some("resume") => {
                "El pedido para reanudar el procesamiento venció; no se hizo nada.".to_string()
            }
            Some("retry") => {
                "El pedido para reintentar el procesamiento venció; no se hizo nada.".to_string()
            }
            _ => "El pedido de control del procesamiento venció; no se hizo nada.".to_string(),
        },
        _ => format!("El pedido «{tool}» venció; no se hizo nada."),
    }
}

fn failed_prefix(tool: &str, args: &serde_json::Value, target: Option<&str>) -> String {
    match tool {
        "collections_delete" => format!("No se pudo borrar {}", collection_phrase(target)),
        "documents_delete" => "No se pudo borrar los documentos".to_string(),
        "documents_text_update" => match target {
            Some(title) => format!("No se pudo actualizar el texto de {}", quote(title)),
            None => "No se pudo actualizar el texto de un documento".to_string(),
        },
        "documents_import" => match target {
            Some(name) => format!("No se pudo importar a la colección {}", quote(name)),
            None => "No se pudo importar los archivos".to_string(),
        },
        "processing_start" => "No se pudo iniciar el procesamiento".to_string(),
        "processing_control" => match args.get("action").and_then(|action| action.as_str()) {
            Some("resume") => "No se pudo reanudar el procesamiento".to_string(),
            Some("retry") => "No se pudo reintentar el procesamiento".to_string(),
            _ => "No se pudo aplicar el control del procesamiento".to_string(),
        },
        _ => format!("No se pudo ejecutar «{tool}»"),
    }
}

/// Follows every pending approval until it is decided or the watch
/// times out, emitting one [`OutcomeEvent`] per approval through `emit`.
/// Polls through `reader`, so the MCP dialog (which decides through the
/// same store) and `approvals_status` (which reads it) both move the
/// watch. A watch that outlives its approvals still ends after
/// `timeout`: leftovers are reported as expired.
pub(crate) async fn watch_pending_approvals<R, F>(
    reader: &R,
    pendings: Vec<PendingApproval>,
    timeout: Duration,
    poll: Duration,
    mut emit: F,
) where
    R: ApprovalReader,
    F: FnMut(OutcomeEvent),
{
    use tokio::time::{sleep, Instant};
    if pendings.is_empty() {
        return;
    }
    let deadline = Instant::now() + timeout;
    let mut open: Vec<PendingApproval> = pendings;
    while !open.is_empty() {
        let mut still: Vec<PendingApproval> = Vec::with_capacity(open.len());
        for pending in open {
            match reader.read(&pending.approval_id) {
                Some(approval) if approval.status != ApprovalStatus::Pending => {
                    let status = outcome_status_wire(&approval.status).to_string();
                    let message = outcome_message(
                        &pending.tool,
                        &pending.args,
                        pending.target.as_deref(),
                        &approval,
                    );
                    emit(OutcomeEvent {
                        approval_id: pending.approval_id,
                        status,
                        message,
                    });
                }
                _ => {
                    if Instant::now() >= deadline {
                        let expired = Approval {
                            status: ApprovalStatus::Expired,
                            failure: None,
                            ..expired_shell(&pending.approval_id)
                        };
                        let message = outcome_message(
                            &pending.tool,
                            &pending.args,
                            pending.target.as_deref(),
                            &expired,
                        );
                        emit(OutcomeEvent {
                            approval_id: pending.approval_id,
                            status: "expired".to_string(),
                            message,
                        });
                    } else {
                        still.push(pending);
                    }
                }
            }
        }
        open = still;
        if !open.is_empty() {
            if Instant::now() >= deadline {
                continue;
            }
            sleep(poll).await;
        }
    }
}

/// The fields [`watch_pending_approvals`] never reads for an expired
/// fallback: only `status` and `failure` matter. Built from the id so the
/// call site stays honest about what timed out.
fn expired_shell(approval_id: &str) -> Approval {
    Approval {
        id: approval_id.to_string(),
        client_id: IN_APP_HELP_CLIENT_ID.to_string(),
        client_name: "Ayuda".to_string(),
        action: crate::automation::approvals::ApprovalAction::Start,
        job_id: String::new(),
        revision: 0,
        operations: Vec::new(),
        collections: Vec::new(),
        files: 0,
        status: ApprovalStatus::Pending,
        created_at: 0,
        expires_at: 0,
        decided_at: None,
        failure: None,
        preview: None,
        text: None,
        import_files: None,
        import_plan: None,
        import_result: None,
        delete_impact: None,
        document_titles: None,
        delete_targets: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::approvals::{ApprovalAction, ApprovalStatus};
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    fn approval_json(id: &str, collections: &[&str], titles: &[&str]) -> String {
        serde_json::json!({
            "status": "pending",
            "id": id,
            "clientId": "in-app-help",
            "clientName": "Ayuda",
            "action": {"kind": "delete_collection", "collectionId": "col_1", "setHash": "h"},
            "jobId": "",
            "revision": 0,
            "operations": ["delete_collection"],
            "collections": collections,
            "files": 2,
            "createdAt": 1,
            "expiresAt": 2,
            "decidedAt": null,
            "failure": null,
            "preview": null,
            "importFiles": null,
            "deleteImpact": null,
            "documentTitles": titles,
            "importResult": null,
        })
        .to_string()
    }

    fn decided(status: ApprovalStatus, failure: Option<&str>) -> Approval {
        Approval {
            id: "apr_1".to_string(),
            client_id: IN_APP_HELP_CLIENT_ID.to_string(),
            client_name: "Ayuda".to_string(),
            action: ApprovalAction::DeleteCollection {
                collection_id: "col_1".to_string(),
                set_hash: "h".to_string(),
            },
            job_id: String::new(),
            revision: 0,
            operations: vec!["delete_collection".to_string()],
            collections: vec!["SIMAPE".to_string()],
            files: 2,
            status,
            created_at: 1,
            expires_at: 2,
            decided_at: Some(3),
            failure: failure.map(str::to_string),
            preview: None,
            text: None,
            import_files: None,
            import_plan: None,
            import_result: None,
            delete_impact: None,
            document_titles: None,
            delete_targets: None,
        }
    }

    #[test]
    fn pending_id_comes_from_a_pending_tool_result() {
        let content = approval_json("apr_1", &["SIMAPE"], &["Acta"]);
        assert_eq!(
            parse_pending_approval_id(&content),
            Some("apr_1".to_string())
        );
    }

    #[test]
    fn non_pending_payloads_carry_no_approval_id() {
        assert_eq!(parse_pending_approval_id("no es json"), None);
        assert_eq!(
            parse_pending_approval_id(&serde_json::json!({"status": "ok"}).to_string()),
            None
        );
        assert_eq!(
            parse_pending_approval_id(&serde_json::json!({"status": "pending"}).to_string()),
            None,
            "pending without an id is not followable"
        );
        assert_eq!(
            parse_pending_approval_id(
                &serde_json::json!({"status": "pending", "id": "  "}).to_string()
            ),
            None
        );
    }

    #[test]
    fn target_names_come_from_the_pending_payload_never_from_uuids() {
        let approval: serde_json::Value =
            serde_json::from_str(&approval_json("apr_1", &["SIMAPE"], &["Acta"])).expect("json");
        assert_eq!(
            extract_target(
                "collections_delete",
                &serde_json::json!({"collectionId": "29189757-1545-411c-b1cc-93132f7aaab7"}),
                &approval
            ),
            Some("SIMAPE".to_string()),
            "the collection TITLE, not the UUID in the args"
        );
        let single: serde_json::Value =
            serde_json::from_str(&approval_json("apr_2", &["SIMAPE"], &["Acta del cabildo"]))
                .expect("json");
        assert_eq!(
            extract_target(
                "documents_delete",
                &serde_json::json!({"itemIds": ["ite_1"]}),
                &single
            ),
            Some("Acta del cabildo".to_string())
        );
        let multi: serde_json::Value =
            serde_json::from_str(&approval_json("apr_3", &["SIMAPE"], &["A", "B", "C"]))
                .expect("json");
        assert_eq!(
            extract_target(
                "documents_delete",
                &serde_json::json!({"itemIds": ["a", "b", "c"]}),
                &multi
            ),
            Some("3 documentos".to_string())
        );
        let empty: serde_json::Value =
            serde_json::from_str(&approval_json("apr_4", &[], &[])).expect("json");
        assert_eq!(
            extract_target(
                "collections_delete",
                &serde_json::json!({"collectionId": "col_x"}),
                &empty
            ),
            None,
            "unknown names stay unknown: the generic fallback wins, never the UUID"
        );
    }

    #[test]
    fn outcome_messages_name_the_target_and_the_decision() {
        let args = serde_json::json!({"collectionId": "col_1"});
        let approved = decided(ApprovalStatus::Approved, None);
        let message = outcome_message("collections_delete", &args, Some("SIMAPE"), &approved);
        assert!(
            message.contains("SIMAPE"),
            "the NAME travels, not the id: {message}"
        );
        assert!(
            message.contains("eliminó") || message.contains("Eliminó"),
            "a clear done sentence: {message}"
        );
        assert!(
            !message.contains("pendiente"),
            "the original pending text is gone: {message}"
        );

        let rejected = decided(ApprovalStatus::Rejected, None);
        let message = outcome_message("collections_delete", &args, Some("SIMAPE"), &rejected);
        assert!(message.contains("SIMAPE"), "{message}");
        assert!(message.contains("Rechazaste"), "{message}");
        assert!(message.contains("no se borró nada"), "{message}");

        let failed = decided(ApprovalStatus::Failed, Some("revision_conflict: cambió"));
        let message = outcome_message("collections_delete", &args, Some("SIMAPE"), &failed);
        assert!(message.contains("SIMAPE"), "{message}");
        assert!(message.contains("No se pudo borrar"), "{message}");
        assert!(
            message.contains("cambió"),
            "the store reason travels: {message}"
        );

        let expired = decided(ApprovalStatus::Expired, None);
        let message = outcome_message("collections_delete", &args, Some("SIMAPE"), &expired);
        assert!(message.contains("venció"), "{message}");
    }

    struct ScriptedReader {
        states: Mutex<HashMap<String, VecDeque<Approval>>>,
    }

    impl ScriptedReader {
        fn new(pairs: Vec<(String, Vec<Approval>)>) -> Self {
            Self {
                states: Mutex::new(
                    pairs
                        .into_iter()
                        .map(|(id, approvals)| (id, approvals.into()))
                        .collect(),
                ),
            }
        }
    }

    impl ApprovalReader for ScriptedReader {
        fn read(&self, approval_id: &str) -> Option<Approval> {
            self.states
                .lock()
                .expect("lock")
                .get_mut(approval_id)
                .and_then(|queue| queue.pop_front())
        }
    }

    fn pending(id: &str, tool: &str, target: Option<&str>) -> PendingApproval {
        PendingApproval {
            approval_id: id.to_string(),
            tool: tool.to_string(),
            args: serde_json::json!({"collectionId": "col_1"}),
            target: target.map(str::to_string),
            request_id: Some("r-1".to_string()),
            conversation_id: None,
        }
    }

    #[tokio::test]
    async fn watcher_emits_approved_rejected_and_failed() {
        let approved = decided(ApprovalStatus::Approved, None);
        let mut rejected = decided(ApprovalStatus::Rejected, None);
        rejected.id = "apr_2".to_string();
        let mut failed = decided(ApprovalStatus::Failed, Some("se rompió"));
        failed.id = "apr_3".to_string();
        let reader = ScriptedReader::new(vec![
            ("apr_1".to_string(), vec![approved]),
            ("apr_2".to_string(), vec![rejected]),
            ("apr_3".to_string(), vec![failed]),
        ]);
        let mut events = Vec::new();
        watch_pending_approvals(
            &reader,
            vec![
                pending("apr_1", "collections_delete", Some("SIMAPE")),
                pending("apr_2", "collections_delete", Some("SIMAPE")),
                pending("apr_3", "collections_delete", Some("SIMAPE")),
            ],
            Duration::from_secs(5),
            Duration::from_millis(1),
            |event| events.push(event),
        )
        .await;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].approval_id, "apr_1");
        assert_eq!(events[0].status, "approved");
        assert!(events[0].message.contains("SIMAPE"));
        assert_eq!(events[1].status, "rejected");
        assert!(events[1].message.contains("Rechazaste"));
        assert_eq!(events[2].status, "failed");
        assert!(events[2].message.contains("se rompió"));
    }

    #[tokio::test]
    async fn watcher_waits_while_pending_and_expires_on_timeout() {
        let id = "apr_9".to_string();
        let pending_row = Approval {
            status: ApprovalStatus::Pending,
            ..decided(ApprovalStatus::Pending, None)
        };
        let reader = ScriptedReader::new(vec![(id.clone(), vec![pending_row])]);
        let mut events = Vec::new();
        watch_pending_approvals(
            &reader,
            vec![pending(&id, "collections_delete", Some("SIMAPE"))],
            Duration::from_millis(20),
            Duration::from_millis(1),
            |event| events.push(event),
        )
        .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, "expired");
        assert!(events[0].message.contains("venció"));
        assert!(events[0].message.contains("SIMAPE"));
    }

    struct MockBridge {
        calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
        answers: Mutex<VecDeque<super::super::agent_llm::ToolOutcome>>,
    }

    type CallLog = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    impl MockBridge {
        #[allow(clippy::type_complexity)]
        fn new(answers: Vec<super::super::agent_llm::ToolOutcome>) -> (Self, CallLog) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    calls: Arc::clone(&calls),
                    answers: Mutex::new(answers.into()),
                },
                calls,
            )
        }
    }

    impl super::super::agent_llm::HelpToolExecutor for MockBridge {
        fn tools(&self) -> Vec<super::super::agent_llm::ToolSpec> {
            Vec::new()
        }

        async fn call(
            &self,
            name: &str,
            args: serde_json::Value,
        ) -> super::super::agent_llm::ToolOutcome {
            self.calls
                .lock()
                .expect("lock")
                .push((name.to_string(), args));
            self.answers.lock().expect("lock").pop_front().unwrap_or(
                super::super::agent_llm::ToolOutcome::Failed {
                    message: "sin respuesta".to_string(),
                },
            )
        }
    }

    #[tokio::test]
    async fn missing_names_are_read_through_the_bridge_never_shown_as_uuids() {
        use super::super::agent_llm::ToolOutcome;
        let page = serde_json::json!({
            "records": [{"id": "col_1", "name": "SIMAPE", "description": null, "itemCount": 1, "createdAt": 1, "updatedAt": 1}],
            "nextCursor": null,
        })
        .to_string();
        let (bridge, _) = MockBridge::new(vec![ToolOutcome::Ok { content: page }]);
        let approval: serde_json::Value =
            serde_json::from_str(&approval_json("apr_1", &[], &[])).expect("json");
        let target = resolve_target(
            &bridge,
            "collections_delete",
            &serde_json::json!({"collectionId": "col_1"}),
            &approval,
        )
        .await;
        assert_eq!(target, Some("SIMAPE".to_string()));
    }

    #[tokio::test]
    async fn an_item_title_comes_from_items_get() {
        use super::super::agent_llm::ToolOutcome;
        let detail = serde_json::json!({
            "id": "ite_1", "title": "Acta del cabildo", "collectionId": "col_1",
            "metadata": null, "createdAt": 1, "updatedAt": 1, "assets": [],
        })
        .to_string();
        let (bridge, _) = MockBridge::new(vec![ToolOutcome::Ok { content: detail }]);
        let approval: serde_json::Value =
            serde_json::from_str(&approval_json("apr_1", &[], &[])).expect("json");
        let target = resolve_target(
            &bridge,
            "documents_text_update",
            &serde_json::json!({"itemId": "ite_1", "assetId": "a", "textHash": "h", "text": "nuevo"}),
            &approval,
        )
        .await;
        assert_eq!(target, Some("Acta del cabildo".to_string()));
    }
}
