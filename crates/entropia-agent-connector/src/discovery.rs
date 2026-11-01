//! The file EntropIA writes while its endpoint runs (D7), read on every message
//! so a restart on a new port is followed without restarting the agent.

use std::path::Path;

use serde::Deserialize;

use crate::error::ConnectorError;

/// File name of the discovery file, in the app's data directory.
pub const DISCOVERY_FILE: &str = "agent-endpoint.json";

/// The protocol version this connector speaks.
pub const PROTOCOL: &str = "entropia-agent/1";

/// The path the endpoint answers on, when the file does not name one.
pub const MCP_PATH: &str = "/mcp";

/// What the app advertises, field for field.
///
/// The fields are all read but only the protocol, the port, the path and the
/// instance key are used; the rest (`pid`, app version, variant, profile) is what
/// makes the file diagnosable by hand and is kept so a malformed one is noticed
/// here.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub protocol: String,
    #[serde(default = "default_mcp_path")]
    pub mcp_path: String,
    pub port: u16,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    /// The proof key this start minted (D12). Read only to verify the instance;
    /// it is never sent over HTTP. Missing on an app that predates D12, which is
    /// why it is optional here and refused in [`parse`].
    #[serde(default)]
    pub instance_key: String,
}

fn default_mcp_path() -> String {
    MCP_PATH.to_string()
}

impl Discovery {
    /// The loopback URL of the endpoint this file advertises.
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.mcp_path)
    }
}

/// Read a discovery file's bytes.
///
/// A file that cannot be understood is the same actionable situation as one
/// that is missing: the app is not there in a state the connector can use.
pub fn parse(bytes: &[u8], source: &Path) -> Result<Discovery, ConnectorError> {
    let discovery: Discovery = serde_json::from_slice(bytes).map_err(|error| {
        ConnectorError::app_absent(format!("{} no se entiende: {error}", source.display()))
    })?;
    if discovery.protocol != PROTOCOL {
        return Err(ConnectorError::protocol_incompatible(&discovery.protocol));
    }
    // Without a usable instance key the connector cannot tell this app from
    // whatever holds the port of a stale file, so it refuses to go further
    // (D12) instead of sending a token it cannot justify sending.
    if !crate::proof::is_hex_64(&discovery.instance_key) {
        return Err(ConnectorError::instance_key_missing());
    }
    if discovery.port == 0 || !is_plain_path(&discovery.mcp_path) {
        return Err(ConnectorError::app_absent(format!(
            "{} no indica un endpoint usable (puerto {}, ruta {:?})",
            source.display(),
            discovery.port,
            discovery.mcp_path
        )));
    }
    Ok(discovery)
}

/// A path the connector is willing to POST to: absolute, no query, no host.
fn is_plain_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains("//")
        && !path.chars().any(char::is_whitespace)
        && !path.contains('\\')
}
