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
/// The credential store cannot be reached from this process (`data.code`
/// `credential_store_unavailable`).
pub const CREDENTIAL_STORE_UNAVAILABLE: i32 = -32007;
/// A sentence for a failure with no dedicated rule: an HTTP status the
/// connector does not know, or a `2xx` that carried no usable answer (W2).
pub const UNEXPECTED_STATUS: i32 = -32000;
/// The line on stdin was not JSON.
pub const PARSE_ERROR: i32 = -32700;

/// What -32005 says to the person. A revoke leaves a registration behind in the
/// assistant, and "pair it again" then sent people to pair yet another client:
/// the same text therefore names both likely causes of a missing credential and
/// the step that actually fixes it (replace, never add a second one). The two
/// ways a lookup fails — nothing stored, or a store that answered an error —
/// keep this same advice; only the diagnostic detail differs.
const CREDENTIAL_MISSING_MESSAGE: &str =
    "No hay una credencial guardada para este cliente en este equipo: lo más probable es que se \
     lo haya revocado, o que se lo haya emparejado en otro equipo. Emparejá un asistente nuevo en \
     EntropIA, Ajustes → Asistentes, y reemplazá la entrada de entropia del asistente con la línea \
     que muestra el panel, sin agregar una segunda.";

/// -32007 on Linux: the Secret Service needs an unlocked keyring, and the
/// session bus variable when the agent scrubbed the environment.
const STORE_UNAVAILABLE_LINUX: &str =
    "No pude acceder al llavero del sistema desde este proceso, así que no puedo leer la \
     credencial del asistente: abrí la sesión del escritorio (el llavero tiene que estar \
     desbloqueado) o, si el agente lanza el conector con un entorno reducido, pasale \
     DBUS_SESSION_BUS_ADDRESS.";

/// -32007 on Windows: the Credential Manager is only reachable from the user's
/// desktop session, so a connector started over SSH or by a service never reads
/// it. D-Bus advice would be meaningless here.
const STORE_UNAVAILABLE_WINDOWS: &str =
    "No pude acceder al llavero del sistema desde este proceso: en Windows es el Administrador \
     de credenciales, que solo está disponible en la sesión de escritorio del usuario, así que un \
     conector lanzado desde SSH o por un servicio no puede leer la credencial del asistente. Abrí \
     el asistente desde la sesión de escritorio y volvé a intentarlo.";

/// -32007 on macOS: an unlocked keychain, and no D-Bus advice.
const STORE_UNAVAILABLE_MACOS: &str =
    "No pude acceder al llavero del sistema desde este proceso, así que no puedo leer la \
     credencial del asistente: el llavero tiene que estar desbloqueado en la sesión del \
     escritorio. Desbloquealo desde Acceso a Llaveros y volvé a intentarlo.";

/// The platform whose credential store a refusal describes. The platform is a
/// parameter of [`ConnectorError::credential_on`] so every text is testable on
/// one machine; [`ConnectorError::credential`] passes this build's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    /// The platform this build runs on.
    pub const fn host() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }

    /// The -32007 sentence to use on this platform.
    fn store_unavailable_message(self) -> &'static str {
        match self {
            Self::Windows => STORE_UNAVAILABLE_WINDOWS,
            Self::MacOs => STORE_UNAVAILABLE_MACOS,
            Self::Linux => STORE_UNAVAILABLE_LINUX,
        }
    }
}

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
    /// in a way that is not "unavailable" (D14), as this build's platform would
    /// word it.
    pub fn credential(error: TokenError) -> Self {
        Self::credential_on(Platform::host(), error)
    }

    /// The same refusal with the -32007 sentence of an explicit platform, so
    /// the Windows and macOS texts are tested on every host (S2).
    pub fn credential_on(platform: Platform, error: TokenError) -> Self {
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
                CREDENTIAL_MISSING_MESSAGE,
                None,
            ),
            TokenError::StoreUnavailable(detail) => Self::new(
                CREDENTIAL_STORE_UNAVAILABLE,
                "credential_store_unavailable",
                platform.store_unavailable_message(),
                Some(detail),
            ),
            TokenError::Failed(reason) => Self::new(
                CREDENTIAL_MISSING,
                "credential_missing",
                CREDENTIAL_MISSING_MESSAGE,
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

    /// A `text/event-stream` reply to a request that carried no JSON event, so
    /// there is no result to forward (same silent-wait hazard as W2).
    pub fn empty_stream() -> Self {
        Self::new(
            UNEXPECTED_STATUS,
            "endpoint_error",
            "EntropIA respondió con un flujo de eventos sin ninguna respuesta legible: volvé a \
             intentarlo; si el problema sigue, actualizá la aplicación y el conector.",
            Some("the event stream carried no JSON event".to_string()),
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

    /// One line for the diagnostic channel: the numeric code, the stable
    /// `data.code` and, when the refusal carries one, the detail. The detail is
    /// the part an assistant never shows (`data.detail`), so it leads; a refusal
    /// without one falls back to its actionable sentence. Never a secret: no
    /// detail here carries the token or the instance key.
    pub fn describe(&self) -> String {
        let context = self.detail.as_deref().unwrap_or(&self.message);
        format!("{} {}: {context}", self.rpc_code, self.code)
    }
}
