//! The failures the connector reports, each with the stable code an agent can
//! branch on and one sentence in Spanish for the person reading its output (S7).

use std::fmt::Display;

use crate::relay::TokenError;

/// The JSON-RPC code of the app being absent (or unreachable).
pub const APP_ABSENT: i32 = -32001;
/// The token was refused: not paired, or revoked.
pub const UNAUTHORIZED: i32 = -32002;
/// The endpoint refused the request itself (loopback guard).
pub const FORBIDDEN: i32 = -32003;
/// The discovery file names another protocol version.
pub const PROTOCOL_INCOMPATIBLE: i32 = -32004;
/// No keyring entry for this client id.
pub const CREDENTIAL_MISSING: i32 = -32005;
/// A sentence for an HTTP status the connector has no rule for.
pub const UNEXPECTED_STATUS: i32 = -32000;
/// The line on stdin was not JSON.
pub const PARSE_ERROR: i32 = -32700;

/// One failure, ready to become a JSON-RPC error response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorError {
    /// The JSON-RPC error code, in the -32000 family (except the parse error).
    pub rpc_code: i32,
    /// The stable `data.code` the agent branches on.
    pub code: &'static str,
    /// The actionable sentence for the person.
    pub message: String,
    /// Technical context for the diagnostic channel, never a secret.
    pub detail: Option<String>,
}

impl ConnectorError {
    fn new(rpc_code: i32, code: &'static str, message: &str, detail: Option<String>) -> Self {
        Self {
            rpc_code,
            code,
            message: message.to_string(),
            detail,
        }
    }

    /// The app is not running: no discovery file, or nothing listening.
    pub fn app_absent(detail: impl Display) -> Self {
        Self::new(
            APP_ABSENT,
            "app_absent",
            "No encuentro EntropIA corriendo en esta máquina: abrí la aplicación y habilitá el \
             asistente en Ajustes → Asistentes.",
            Some(detail.to_string()),
        )
    }

    /// 401: the client is not paired, or its credential was revoked.
    pub fn unauthorized() -> Self {
        Self::new(
            UNAUTHORIZED,
            "unauthorized",
            "EntropIA rechazó la credencial de este asistente: emparejalo de nuevo en Ajustes → \
             Asistentes.",
            None,
        )
    }

    /// 403: the endpoint refused the request regardless of the credential.
    pub fn forbidden() -> Self {
        Self::new(
            FORBIDDEN,
            "forbidden",
            "EntropIA rechazó esta conexión: el asistente solo puede usar el endpoint local de \
             EntropIA en este equipo.",
            None,
        )
    }

    /// The discovery file announces another protocol version.
    pub fn protocol_incompatible(found: &str) -> Self {
        Self::new(
            PROTOCOL_INCOMPATIBLE,
            "protocol_incompatible",
            "La versión de este conector no coincide con la de EntropIA: actualizá la aplicación y \
             el conector.",
            Some(format!("discovery protocol {found:?}")),
        )
    }

    /// No keyring entry (or an unreadable keyring) for this client id.
    pub fn credential(error: TokenError) -> Self {
        let detail = match error {
            TokenError::Missing => None,
            TokenError::Failed(reason) => Some(reason),
        };
        Self::new(
            CREDENTIAL_MISSING,
            "credential_missing",
            "No hay una credencial guardada para este cliente: emparejalo en EntropIA, Ajustes → \
             Asistentes.",
            detail,
        )
    }

    /// A stdin line that is not JSON.
    pub fn parse_error(error: impl Display) -> Self {
        Self::new(
            PARSE_ERROR,
            "parse_error",
            "El conector recibió una línea que no es JSON-RPC válido.",
            Some(error.to_string()),
        )
    }

    /// A status no rule covers (404, 500, …).
    pub fn unexpected_status(status: u16) -> Self {
        Self::new(
            UNEXPECTED_STATUS,
            "endpoint_error",
            "EntropIA respondió con un error inesperado.",
            Some(format!("HTTP {status}")),
        )
    }

    /// One line for the diagnostic channel.
    pub fn describe(&self) -> String {
        match &self.detail {
            Some(detail) => format!("{}: {} ({detail})", self.code, self.message),
            None => format!("{}: {}", self.code, self.message),
        }
    }
}
