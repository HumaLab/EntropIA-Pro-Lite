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
/// The fallback token file is readable by group or others (D14).
pub const CREDENTIAL_INSECURE: i32 = -32006;
/// A sentence for a failure with no dedicated rule: an HTTP status the
/// connector does not know, or a `2xx` that carried no usable answer (W2).
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

    /// The discovery file announces another protocol version, or does not carry
    /// the instance key that version includes (D12).
    pub fn protocol_incompatible(found: &str) -> Self {
        Self::new(
            PROTOCOL_INCOMPATIBLE,
            "protocol_incompatible",
            "La versión de este conector no coincide con la de EntropIA: actualizá la aplicación y \
             el conector.",
            Some(format!("discovery protocol {found:?}")),
        )
    }

    /// The discovery file has no usable instance key (D12): an app old enough to
    /// predate the proof cannot be verified, so the connector refuses it as a
    /// version mismatch instead of trusting the port.
    pub fn instance_key_missing() -> Self {
        Self::new(
            PROTOCOL_INCOMPATIBLE,
            "protocol_incompatible",
            "La versión de este conector no coincide con la de EntropIA: actualizá la aplicación y \
             el conector.",
            Some("discovery has no usable instanceKey".to_string()),
        )
    }

    /// The endpoint could not prove it holds the discovery file's instance key
    /// (D12): the credential is never sent to it.
    pub fn instance_unverified(detail: impl Display) -> Self {
        Self::new(
            APP_ABSENT,
            "app_absent",
            "No pude verificar que EntropIA sea la aplicación detrás de esa conexión: abrí la \
             aplicación y volvé a intentarlo.",
            Some(detail.to_string()),
        )
    }

    /// No token anywhere for this client id, or a credential store that failed
    /// in a way that is not "unavailable" (D14).
    pub fn credential(error: TokenError) -> Self {
        match error {
            TokenError::Insecure(detail) => Self::new(
                CREDENTIAL_INSECURE,
                "credential_insecure",
                "La credencial guardada en este equipo la puede leer cualquier usuario: corregí los \
                 permisos del archivo de credenciales o volvé a emparejar el asistente en \
                 EntropIA, Ajustes → Asistentes.",
                Some(detail),
            ),
            TokenError::Missing => Self::new(
                CREDENTIAL_MISSING,
                "credential_missing",
                "No hay una credencial guardada para este cliente: emparejalo en EntropIA, Ajustes → \
                 Asistentes.",
                None,
            ),
            TokenError::Failed(reason) => Self::new(
                CREDENTIAL_MISSING,
                "credential_missing",
                "No hay una credencial guardada para este cliente: emparejalo en EntropIA, Ajustes → \
                 Asistentes.",
                Some(reason),
            ),
        }
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

    /// A request the endpoint accepted but answered with nothing (W2). An agent
    /// waiting for that result would wait forever, so the silence becomes a
    /// JSON-RPC error like any other failure.
    pub fn empty_reply() -> Self {
        Self::new(
            UNEXPECTED_STATUS,
            "endpoint_error",
            "EntropIA aceptó la petición pero no devolvió ninguna respuesta: volvé a intentarlo; \
             si el problema sigue, revisá que la aplicación siga abierta.",
            Some("HTTP 2xx with an empty body".to_string()),
        )
    }

    /// A 2xx body that is not JSON where a result was expected (W2).
    pub fn invalid_reply(error: impl Display) -> Self {
        Self::new(
            UNEXPECTED_STATUS,
            "endpoint_error",
            "EntropIA devolvió una respuesta que el conector no pudo leer: volvé a intentarlo; si \
             el problema sigue, actualizá la aplicación y el conector.",
            Some(format!("the response body is not JSON: {error}")),
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
