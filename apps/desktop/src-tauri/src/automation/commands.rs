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
