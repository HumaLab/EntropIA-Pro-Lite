//! The Tauri commands behind Configuración → Asistentes (D11).
//!
//! They are thin on purpose: [`super::access::AgentAccess`] owns who may connect
//! and [`super::server::AgentEndpointState`] owns whether anything listens, so
//! every command here assembles a status or hands one call through. Errors stay
//! `String`, like every other command in the app, so the panel can show what the
//! backend said without a second translation layer.
//!
//! `set_enabled` persists first and syncs the endpoint second (D8): a crash
//! between the two leaves the file saying "enabled" with nothing listening,
//! which the next startup fixes, instead of serving an endpoint the file says is
//! off. No command ever returns a token: the client view has never carried one.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::access::{AgentAccess, ClientView, Grants};
use super::approvals::Approval;
use super::connector::{current_location, ConnectorLocation};
use super::server::AgentEndpointState;

/// `agent_access_status`'s payload (D11): the persisted switch, what the
/// endpoint is doing right now, and the paired clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusView {
    pub enabled: bool,
    pub running: bool,
    /// The port the endpoint listens on, while it listens.
    pub port: Option<u16>,
    pub clients: Vec<ClientView>,
    /// Folders assistants may import from.
    pub import_folders: Vec<String>,
}

/// `agent_client_create`'s payload: the new client and the status after adding
/// it, so the panel needs no second round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedClientView {
    pub client: ClientView,
    pub status: AgentStatusView,
}

/// Assemble the status from the two owners. `running` is `port.is_some()`: the
/// port exists exactly while the listener does.
pub fn status(
    access: &AgentAccess,
    endpoint: &AgentEndpointState,
) -> Result<AgentStatusView, String> {
    let view = access.view()?;
    let port = endpoint.port();
    Ok(AgentStatusView {
        enabled: view.enabled,
        running: port.is_some(),
        port,
        clients: view.clients,
        import_folders: view.import_folders,
    })
}

/// Flip the switch: persist, then start or stop the endpoint. A start failure is
/// returned to the caller, which shows it; the persisted flag stays as asked.
pub async fn set_enabled(
    access: &Arc<AgentAccess>,
    endpoint: &AgentEndpointState,
    enabled: bool,
) -> Result<AgentStatusView, String> {
    access.set_enabled(enabled)?;
    endpoint.sync(access).await?;
    status(access, endpoint)
}

/// Pair a client and report the status it left behind. Creating a client never
/// starts or stops the endpoint: the switch owns that.
pub fn create_client(
    access: &AgentAccess,
    endpoint: &AgentEndpointState,
    name: &str,
) -> Result<CreatedClientView, String> {
    let client = access.create_client(name)?;
    Ok(CreatedClientView {
        client,
        status: status(access, endpoint)?,
    })
}

/// Revoke a client and report the status after it. Revoking a client that is
/// already gone is a no-op, not an error: the outcome the caller asked for
/// holds either way.
pub fn revoke_client(
    access: &AgentAccess,
    endpoint: &AgentEndpointState,
    client_id: &str,
) -> Result<AgentStatusView, String> {
    access.revoke_client(client_id)?;
    status(access, endpoint)
}

/// Apply the owner's grants to one client and report the status after it.
/// Unlike revoking, an unknown client is an error: there is nothing to grant.
pub fn set_client_grants(
    access: &AgentAccess,
    endpoint: &AgentEndpointState,
    client_id: &str,
    grants: Grants,
) -> Result<AgentStatusView, String> {
    access.set_grants(client_id, grants)?;
    status(access, endpoint)
}

/// Enable a folder for assistant imports and report the status after it.
pub fn add_import_folder(
    access: &AgentAccess,
    endpoint: &AgentEndpointState,
    path: &str,
) -> Result<AgentStatusView, String> {
    access.add_import_folder(path)?;
    status(access, endpoint)
}

/// Stop assistant imports from a folder and report the status after it.
pub fn remove_import_folder(
    access: &AgentAccess,
    endpoint: &AgentEndpointState,
    path: &str,
) -> Result<AgentStatusView, String> {
    access.remove_import_folder(path)?;
    status(access, endpoint)
}

/// The requests waiting for the owner, oldest first.
pub fn pending_approvals(endpoint: &AgentEndpointState) -> Vec<Approval> {
    endpoint.approvals().pending()
}

/// The owner's answer to one request. Approving runs the action once on a
/// fresh connection to `db_path`, fenced by the revision the assistant saw.
/// An import reads the folders enabled at this moment, so a folder the owner
/// removed meanwhile is not read.
pub fn decide_approval(
    endpoint: &AgentEndpointState,
    access: &AgentAccess,
    db_path: &std::path::Path,
    approval_id: &str,
    approve: bool,
) -> Result<Approval, String> {
    let conn = crate::processing::commands::open_ready(db_path)?;
    let context = super::approvals::ExecContext {
        // The archive's data directory holds the database and `assets/`.
        data_dir: db_path.parent().map(std::path::Path::to_path_buf),
        folders: access.import_folders(),
    };
    let decided = endpoint
        .approvals()
        .decide_with(&conn, approval_id, approve, &context)?;
    let decision = match decided.status {
        super::approvals::ApprovalStatus::Approved => "approved",
        super::approvals::ApprovalStatus::Failed => "failed",
        _ => "rejected",
    };
    // An applied text replacement changed an item the views may show.
    if decided.status == super::approvals::ApprovalStatus::Approved {
        if let super::approvals::ApprovalAction::ReplaceText { item_id, .. } = &decided.action {
            let collection_id: Option<String> = conn
                .query_row(
                    "SELECT collection_id FROM items WHERE id = ?1",
                    [item_id],
                    |row| row.get(0),
                )
                .ok();
            if let Some(collection_id) = collection_id {
                endpoint
                    .corpus_events()
                    .notify(super::corpus_tools::CorpusChange {
                        kind: "item",
                        id: item_id.clone(),
                        collection_id,
                    });
            }
        }
        // Imported items appear in the sidebar and their collection.
        if let Some(result) = &decided.import_result {
            for imported in &result.imported {
                endpoint
                    .corpus_events()
                    .notify(super::corpus_tools::CorpusChange {
                        kind: "item",
                        id: imported.item_id.clone(),
                        collection_id: result.collection_id.clone(),
                    });
            }
        }
    }
    let target_id = match &decided.action {
        super::approvals::ApprovalAction::ReplaceText { item_id, .. } => item_id,
        super::approvals::ApprovalAction::ImportFiles { collection_id, .. } => collection_id,
        _ => &decided.job_id,
    };
    endpoint
        .audit_log()
        .append(&super::audit::AuditLine::owner_decision(
            &decided.client_id,
            &decided.client_name,
            &decided.id,
            target_id,
            decision,
        ));
    Ok(decided)
}

#[tauri::command]
pub fn agent_access_status(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
) -> Result<AgentStatusView, String> {
    status(access.inner(), endpoint.inner())
}

#[tauri::command]
pub async fn agent_access_set_enabled(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
    enabled: bool,
) -> Result<AgentStatusView, String> {
    let access = access.inner().clone();
    let endpoint = endpoint.inner().clone();
    set_enabled(&access, &endpoint, enabled).await
}

#[tauri::command]
pub fn agent_client_create(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
    name: String,
) -> Result<CreatedClientView, String> {
    create_client(access.inner(), endpoint.inner(), &name)
}

#[tauri::command]
pub fn agent_client_revoke(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
    client_id: String,
) -> Result<AgentStatusView, String> {
    revoke_client(access.inner(), endpoint.inner(), &client_id)
}

#[tauri::command]
pub fn agent_client_set_grants(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
    client_id: String,
    grants: Grants,
) -> Result<AgentStatusView, String> {
    set_client_grants(access.inner(), endpoint.inner(), &client_id, grants)
}

#[tauri::command]
pub fn agent_import_folder_add(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
    path: String,
) -> Result<AgentStatusView, String> {
    add_import_folder(access.inner(), endpoint.inner(), &path)
}

#[tauri::command]
pub fn agent_import_folder_remove(
    access: State<'_, Arc<AgentAccess>>,
    endpoint: State<'_, Arc<AgentEndpointState>>,
    path: String,
) -> Result<AgentStatusView, String> {
    remove_import_folder(access.inner(), endpoint.inner(), &path)
}

#[tauri::command]
pub fn agent_approvals_pending(endpoint: State<'_, Arc<AgentEndpointState>>) -> Vec<Approval> {
    pending_approvals(endpoint.inner())
}

#[tauri::command]
pub async fn agent_approval_decide(
    endpoint: State<'_, Arc<AgentEndpointState>>,
    access: State<'_, Arc<AgentAccess>>,
    db: State<'_, crate::db::state::AppDbState>,
    approval_id: String,
    approve: bool,
) -> Result<Approval, String> {
    let endpoint = endpoint.inner().clone();
    let access = access.inner().clone();
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        decide_approval(&endpoint, &access, &db_path, &approval_id, approve)
    })
    .await
    .map_err(|e| format!("the approval task did not finish: {e}"))?
}

/// Where the bundled connector is, so the panel can name an absolute path in
/// every recipe (D15). The decision lives in [`super::connector`]; this only
/// exposes it, and it takes no state: the answer describes the installation, not
/// a session.
#[tauri::command]
pub fn agent_connector_location() -> ConnectorLocation {
    current_location()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::access::tests::MemorySecrets;
    use crate::automation::access::Scope;
    use crate::automation::server::DISCOVERY_FILE;

    struct Harness {
        dir: tempfile::TempDir,
        secrets: Arc<MemorySecrets>,
        access: Arc<AgentAccess>,
        endpoint: AgentEndpointState,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.endpoint.stop();
        }
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let access = Arc::new(AgentAccess::load(dir.path(), secrets.clone()));
        let endpoint = AgentEndpointState::new(
            dir.path().to_path_buf(),
            dir.path().join("archive.db"),
            dir.path().join("cache"),
        );
        Harness {
            dir,
            secrets,
            access,
            endpoint,
        }
    }

    #[test]
    fn a_disabled_endpoint_reports_no_port_and_lists_the_paired_clients() {
        let harness = harness();
        assert_eq!(
            status(&harness.access, &harness.endpoint).unwrap(),
            AgentStatusView {
                enabled: false,
                running: false,
                port: None,
                clients: vec![],
                import_folders: vec![],
            }
        );
        let client = harness.access.create_client("Claude Code").unwrap();
        let view = status(&harness.access, &harness.endpoint).unwrap();
        assert!(!view.enabled);
        assert!(!view.running, "a paired client does not start the endpoint");
        assert_eq!(view.port, None);
        assert_eq!(view.clients, vec![client]);
    }

    #[tokio::test]
    async fn enabling_persists_then_starts_the_endpoint_and_reports_its_port() {
        let harness = harness();
        let view = set_enabled(&harness.access, &harness.endpoint, true)
            .await
            .unwrap();
        assert!(view.enabled);
        assert!(view.running);
        let port = view.port.expect("a running endpoint has a port");
        assert_eq!(Some(port), harness.endpoint.port());
        assert!(
            harness.dir.path().join(DISCOVERY_FILE).exists(),
            "the connector finds the endpoint through the discovery file"
        );
        // Persisted before the endpoint started: a reload sees the same switch.
        let reloaded = AgentAccess::load(harness.dir.path(), harness.secrets.clone());
        assert!(reloaded.is_enabled());
    }

    #[tokio::test]
    async fn disabling_stops_the_endpoint_and_reports_it_stopped() {
        let harness = harness();
        set_enabled(&harness.access, &harness.endpoint, true)
            .await
            .unwrap();
        let view = set_enabled(&harness.access, &harness.endpoint, false)
            .await
            .unwrap();
        assert!(!view.enabled);
        assert!(!view.running);
        assert_eq!(view.port, None);
        assert!(!harness.dir.path().join(DISCOVERY_FILE).exists());
    }

    #[tokio::test]
    async fn creating_a_client_returns_it_read_only_with_the_current_status() {
        let harness = harness();
        let running = set_enabled(&harness.access, &harness.endpoint, true)
            .await
            .unwrap();
        let created = create_client(&harness.access, &harness.endpoint, "  Claude Code ").unwrap();
        assert_eq!(created.client.name, "Claude Code", "the name is trimmed");
        assert!(created.client.id.starts_with("cl_"));
        assert_eq!(created.client.scopes, vec![Scope::Read]);
        assert_eq!(created.status.port, running.port);
        assert_eq!(created.status.clients, vec![created.client.clone()]);
        // The token lives in the store, never in what the UI receives (S4).
        let token = harness
            .secrets
            .token(&created.client.id)
            .expect("stored token");
        let json = serde_json::to_string(&created).unwrap();
        assert!(!json.contains(&token), "a status must never carry a token");
        assert!(!json.contains("tokenSha256"), "nor its hash");
        assert!(
            json.contains("\"tokenStore\":\"keyring\""),
            "the panel shows where the token is stored: {json}"
        );
    }

    #[test]
    fn an_invalid_name_is_refused_and_adds_no_client() {
        let harness = harness();
        let error = create_client(&harness.access, &harness.endpoint, "   ").unwrap_err();
        assert!(error.starts_with("invalid_client_name"), "{error}");
        assert!(status(&harness.access, &harness.endpoint)
            .unwrap()
            .clients
            .is_empty());
    }

    #[test]
    fn granting_scopes_returns_the_status_with_the_updated_client() {
        let harness = harness();
        let created = create_client(&harness.access, &harness.endpoint, "Codex").unwrap();
        let view = set_client_grants(
            &harness.access,
            &harness.endpoint,
            &created.client.id,
            Grants {
                local_write: true,
                remote_spend: false,
            },
        )
        .unwrap();
        assert_eq!(view.clients.len(), 1);
        assert_eq!(view.clients[0].scopes, vec![Scope::Read, Scope::LocalWrite]);
        let error = set_client_grants(
            &harness.access,
            &harness.endpoint,
            "cl_missing",
            Grants::default(),
        )
        .unwrap_err();
        assert!(error.starts_with("unknown_client"), "{error}");
    }

    #[test]
    fn the_owner_lists_and_answers_an_assistants_request() {
        use crate::automation::approvals::{ApprovalAction, ApprovalRequest, ApprovalStatus};
        use crate::automation::test_archive::{seeded_archive, ALFA};
        use crate::processing::commands as batches;

        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_archive(dir.path());
        let endpoint = AgentEndpointState::new(
            dir.path().to_path_buf(),
            db_path.clone(),
            dir.path().join("cache"),
        );
        let conn = batches::open_ready(&db_path).unwrap();
        let job = batches::prepare_batch(
            &conn,
            "req-1",
            &[ALFA.to_string()],
            &[],
            &["ocr".to_string()],
        )
        .unwrap()
        .batch_id;
        conn.execute(
            "UPDATE processing_batches SET state = 'ready', planning_done = 1 WHERE id = ?1",
            [&job],
        )
        .unwrap();
        let revision = batches::batch_snapshot(&conn, &job).unwrap().revision;
        let asked = endpoint.approvals().request(ApprovalRequest {
            client_id: "cl_a".into(),
            client_name: "Claude Code".into(),
            action: ApprovalAction::Start,
            job_id: job.clone(),
            revision,
            operations: vec!["ocr".into()],
            collections: vec!["Alfa".into()],
            files: 2,
            preview: None,
            text: None,
            import_plan: None,
        });

        assert_eq!(pending_approvals(&endpoint), vec![asked.clone()]);
        let access = AgentAccess::load(dir.path(), Arc::new(MemorySecrets::default()));
        let decided = decide_approval(&endpoint, &access, &db_path, &asked.id, true).unwrap();
        assert_eq!(decided.status, ApprovalStatus::Approved);
        assert_eq!(
            batches::batch_snapshot(&conn, &job).unwrap().desired_state,
            "run"
        );
        assert!(pending_approvals(&endpoint).is_empty());
        let json = serde_json::to_string(&decided).unwrap();
        assert!(!json.contains("eat_"), "an approval never carries a token");

        // S9: the owner's answer is in the audit trail, ids only.
        let trail = std::fs::read_to_string(
            dir.path()
                .join("cache")
                .join("logs")
                .join(crate::automation::audit::AUDIT_FILE),
        )
        .expect("the decision was audited");
        let line: serde_json::Value =
            serde_json::from_str(trail.lines().last().expect("one line")).unwrap();
        assert_eq!(line["tool"], "owner_decision");
        assert_eq!(line["clientId"], "cl_a");
        assert_eq!(line["status"], "ok");
        assert_eq!(line["decision"], "approved");
        assert_eq!(line["objectIds"], serde_json::json!([asked.id, job]));
    }

    #[test]
    fn import_folders_are_added_and_removed_through_the_status() {
        let harness = harness();
        let inbox = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(inbox.path()).unwrap();
        let real = real.to_string_lossy().to_string();
        let added = add_import_folder(
            &harness.access,
            &harness.endpoint,
            &inbox.path().to_string_lossy(),
        )
        .unwrap();
        assert_eq!(added.import_folders, vec![real.clone()]);
        let refused = add_import_folder(&harness.access, &harness.endpoint, "").unwrap_err();
        assert!(refused.starts_with("invalid_folder:"), "{refused}");
        let removed = remove_import_folder(&harness.access, &harness.endpoint, &real).unwrap();
        assert!(removed.import_folders.is_empty());
        let json = serde_json::to_string(&added).unwrap();
        assert!(json.contains("\"importFolders\""), "{json}");
    }

    #[tokio::test]
    async fn revoking_drops_the_client_and_refuses_a_copied_token() {
        let harness = harness();
        set_enabled(&harness.access, &harness.endpoint, true)
            .await
            .unwrap();
        let created = create_client(&harness.access, &harness.endpoint, "Codex").unwrap();
        let copied = harness.secrets.token(&created.client.id).unwrap();
        let view = revoke_client(&harness.access, &harness.endpoint, &created.client.id).unwrap();
        assert!(view.clients.is_empty());
        assert_eq!(harness.access.authenticate(&copied), None);
        assert_eq!(harness.secrets.token(&created.client.id), None);
        // Revoking what is already gone is a no-op, not an error.
        assert!(revoke_client(&harness.access, &harness.endpoint, &created.client.id).is_ok());
    }
}
