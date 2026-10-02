//! HTTP transport for the cloud-sync client (PROTOCOL.md). Defines the
//! [`SyncApi`] trait (the wire contract the engine and push/pull paths consume)
//! plus the [`HttpSyncApi`] reqwest implementation and the request/response
//! DTOs. The trait is generic-friendly so the push/blob slices can be tested
//! against an in-memory mock without spinning up a server.
//!
//! House rules enforced here (DESIGN §8, PROTOCOL "Transporte"):
//! - TLS is mandatory: a non-`https` `server_url` is rejected unless the host is
//!   `127.0.0.1` / `::1` / `localhost`. Validated at config-time
//!   ([`validate_server_url`]) AND at use-time (inside [`HttpSyncApi::new`]).
//! - The Bearer token and the `X-Schema-Tag` header are attached to every
//!   authenticated request; the token is NEVER logged (DESIGN §8).
//! - `X-Sync-Capabilities` carries `web-capture-v1` on ordinary push/pull
//!   requests (web captures are ordinary rows) and the aggregate token on the
//!   explicit writing-envelope-v1 / research-envelope-v1 methods, which send
//!   only their own token.
//!
//! Several DTO fields and trait methods (devices, revoke, usage, delete_account,
//! pull, blob_get, and the pull-response cursor fields) are consumed by the
//! engine and pull/apply slices (next slices); here they exist for the wire
//! contract and are only partially exercised. The module-level
//! `allow(dead_code)` is removed once those slices wire them up (same convention
//! as the C1 foundations).
#![allow(dead_code)]

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Connect timeout for every request (house style mirrors the AssemblyAI/GLM
/// clients which cap connection setup so a stalled socket cannot wedge a worker).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Default per-request timeout for the small JSON endpoints.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Generous timeout for blob HEAD/PUT/GET, which can move large files.
const BLOB_TIMEOUT: Duration = Duration::from_secs(600);

/// HTTP header carrying the local migration head (PROTOCOL "schema_tag").
const SCHEMA_TAG_HEADER: &str = "X-Schema-Tag";
/// Optional sync capability header (PROTOCOL "Negociación de capacidades de sync").
const SYNC_CAPABILITIES_HEADER: &str = "X-Sync-Capabilities";
/// Exact, case-sensitive capability token for writing aggregate envelopes.
pub const WRITING_ENVELOPE_V1_CAPABILITY: &str = "writing-envelope-v1";
/// Exact, case-sensitive capability token for research aggregate envelopes.
pub const RESEARCH_ENVELOPE_V1_CAPABILITY: &str = "research-envelope-v1";
/// Exact, case-sensitive capability token for saved web captures. Unlike the
/// aggregates above, these are ordinary rows: every ordinary push/pull sends it.
pub const WEB_CAPTURE_V1_CAPABILITY: &str = "web-capture-v1";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Uniform error type for the sync transport. Surfaced as `String` at the Tauri
/// command boundary (house rule: commands return `Result<T, String>`), but kept
/// structured internally so the engine can branch on `Api { status, .. }` codes
/// (e.g. 426 → "update the app", 507 → "server storage full").
#[derive(Debug)]
pub enum SyncError {
    /// `server_url` failed the TLS rule (non-https to a non-loopback host) or is
    /// otherwise malformed.
    InvalidUrl(String),
    /// Transport-level failure (DNS, connect, timeout, TLS handshake, broken
    /// stream). Maps to the engine's `offline` state with backoff.
    Network(String),
    /// A structured `{ "error": { code, message } }` body with its HTTP status.
    Api {
        status: u16,
        code: String,
        message: String,
    },
    /// The body could not be (de)serialized as expected.
    Decode(String),
}

impl SyncError {
    /// The HTTP status if this is an [`SyncError::Api`], else `None`. Used by the
    /// engine to branch on 4xx codes without re-parsing the message.
    #[allow(dead_code)]
    pub fn status(&self) -> Option<u16> {
        match self {
            SyncError::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The stable error code if this is an [`SyncError::Api`], else `None`.
    #[allow(dead_code)]
    pub fn api_code(&self) -> Option<&str> {
        match self {
            SyncError::Api { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::InvalidUrl(m) => write!(f, "invalid server url: {m}"),
            SyncError::Network(m) => write!(f, "network error: {m}"),
            SyncError::Api {
                status,
                code,
                message,
            } => write!(f, "api error {status} ({code}): {message}"),
            SyncError::Decode(m) => write!(f, "decode error: {m}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<SyncError> for String {
    fn from(error: SyncError) -> Self {
        error.to_string()
    }
}

/// The wire error envelope (`{ "error": { "code", "message" } }`, PROTOCOL).
#[derive(Debug, Deserialize)]
struct WireError {
    error: Option<WireErrorBody>,
}

#[derive(Debug, Deserialize)]
struct WireErrorBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

// ---------------------------------------------------------------------------
// TLS / URL validation (PROTOCOL "Transporte", DESIGN §8)
// ---------------------------------------------------------------------------

/// Returns true when `host` is a loopback address for which plain `http://` is
/// tolerated (PROTOCOL "Transporte"). Everything else demands TLS.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost" | "[::1]")
}

/// Validates a `server_url` against the TLS rule. `https` is always accepted;
/// `http` only for loopback hosts. Returns the trimmed, trailing-slash-stripped
/// base URL on success. Called at config-time and at use-time so a stored URL
/// can never be used to ship credentials over cleartext.
pub fn validate_server_url(url: &str) -> Result<String, SyncError> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(SyncError::InvalidUrl("empty server url".to_string()));
    }

    let (scheme, rest) = trimmed
        .split_once("://")
        .ok_or_else(|| SyncError::InvalidUrl(format!("missing scheme in '{trimmed}'")))?;

    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(SyncError::InvalidUrl(format!(
            "unsupported scheme '{scheme}' (only http/https)"
        )));
    }

    // Authority is everything up to the first '/', '?' or '#'.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("").to_string();
    if authority.is_empty() {
        return Err(SyncError::InvalidUrl(format!(
            "missing host in '{trimmed}'"
        )));
    }

    // Strip optional userinfo and port to isolate the host for the loopback check.
    let host_port = authority.rsplit('@').next().unwrap_or(&authority);
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        // Bracketed IPv6 literal, e.g. [::1]:8787 → keep the brackets so the
        // loopback check matches "[::1]".
        match stripped.split_once(']') {
            Some((inner, _)) => format!("[{inner}]"),
            None => host_port.to_string(),
        }
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
            .to_string()
    };
    let host_for_match = host.trim_start_matches('[').trim_end_matches(']');

    if scheme == "http" && !is_loopback_host(&host) && !is_loopback_host(host_for_match) {
        return Err(SyncError::InvalidUrl(format!(
            "refusing http:// to non-loopback host '{host}' — TLS is mandatory"
        )));
    }

    Ok(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// DTOs (PROTOCOL "Endpoints"). All response structs tolerate unknown fields by
// default (serde drops them) so additive `/v1` changes don't break the client.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub account_id: String,
}

#[derive(Debug, Serialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    pub device_name: String,
    pub platform: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub account_id: String,
    pub device_id: String,
    pub device_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub last_seen_at: i64,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub current: bool,
}

#[derive(Debug, Deserialize)]
pub struct DevicesResponse {
    #[serde(default)]
    pub devices: Vec<DeviceInfo>,
}

#[derive(Debug, Serialize)]
pub struct DeleteAccountRequest {
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageResponse {
    #[serde(default)]
    pub rows: i64,
    #[serde(default)]
    pub blobs_count: i64,
    #[serde(default)]
    pub blobs_bytes: i64,
    #[serde(default)]
    pub quota_bytes: i64,
    /// Nombre del plan / tipo de suscripción (`Free`, `5 GB`, …); `None` si la cuenta
    /// no tiene plan asignado o el servidor no lo reporta.
    #[serde(default)]
    pub plan_name: Option<String>,
    /// Vencimiento de la suscripción en ms (`None` = nunca vence; p.ej. cuentas Free o
    /// servidor viejo que no reporta el campo). Aditivo (PROTOCOL `/v1/usage`).
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// Notificaciones in-app NO leídas de la cuenta (badge del inbox). `0` por defecto si
    /// el servidor no lo reporta. Aditivo (PROTOCOL `/v1/usage`).
    #[serde(default)]
    pub unread_notifications: i64,
    /// Nombre del plan solicitado si hay una solicitud de cambio de plan en estado
    /// `pending`; `None` si no hay ninguna o el servidor no lo reporta. Aditivo. El cliente
    /// lo usa para mostrar "Solicitud en revisión" de forma persistente.
    #[serde(default)]
    pub pending_plan_request: Option<String>,
}

/// Un plan del catálogo (PROTOCOL `GET /v1/plans`). Replica `PlanView` del servidor.
/// El cliente lo usa para poblar el `<select>` del modal "solicitar upgrade".
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanCatalogItem {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Cuota del plan en bytes; `0` = ilimitada.
    #[serde(default)]
    pub quota_bytes: i64,
    /// Precio en centavos; `0` = gratuito.
    #[serde(default)]
    pub price_cents: i64,
    #[serde(default)]
    pub currency: String,
    /// Período de cobro (`monthly` | `yearly` | `none`).
    #[serde(default)]
    pub period: String,
    /// Descripción del plan (`None` si no tiene).
    #[serde(default)]
    pub description: Option<String>,
    /// `true` solo en el plan actual de la cuenta autenticada.
    #[serde(default)]
    pub is_current: bool,
}

/// Envoltura de `GET /v1/plans`.
#[derive(Debug, Deserialize)]
struct PlansResponse {
    #[serde(default)]
    plans: Vec<PlanCatalogItem>,
}

/// Una notificación in-app del inbox del usuario (PROTOCOL `GET /v1/notifications`).
/// Replica `NotificationView` del servidor: `{ id, kind, severity, title, body,
/// created_at, read_at }`. `category` no viaja hoy en el wire pero se tolera con
/// `#[serde(default)]` por si una versión futura del servidor lo agrega (aditivo).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationItem {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    /// Categoría server-side (`transactional` | `reminder` | `operator`). No la manda el
    /// servidor en el inbox del usuario hoy; tolerada para compat futura.
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: i64,
    /// `None` si no fue leída todavía.
    #[serde(default)]
    pub read_at: Option<i64>,
}

/// Envoltura de `GET /v1/notifications`.
#[derive(Debug, Deserialize)]
struct NotificationsResponse {
    #[serde(default)]
    notifications: Vec<NotificationItem>,
}

/// Body de `POST /v1/plan-change-request`.
#[derive(Debug, Serialize)]
pub struct PlanChangeRequestBody {
    pub requested_plan_id: String,
    /// Nota opcional del usuario para el equipo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Solicitud de cambio de plan creada (PROTOCOL `POST /v1/plan-change-request`).
/// Replica `PlanChangeRequestView` del servidor.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanChangeRequestResponse {
    pub id: String,
    /// Plan actual de la cuenta al momento de pedir (`None` = sin plan / Free).
    #[serde(default)]
    pub current_plan_id: Option<String>,
    #[serde(default)]
    pub requested_plan_id: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: i64,
}

/// One outbound change in a push batch (PROTOCOL `POST /v1/sync/push`).
#[derive(Debug, Clone, Serialize)]
pub struct PushChange {
    pub table: String,
    pub row_id: String,
    /// `"upsert"` | `"delete"`.
    pub op: String,
    /// ms, already corrected by the clock offset (PROTOCOL "Reloj").
    pub changed_at: i64,
    /// Last server version seen of this row; `0` = never seen.
    pub base_seq: i64,
    /// Row payload as a JSON object for `upsert`; `None` (null) for `delete`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct PushRequest {
    pub changes: Vec<PushChange>,
}

/// One per-row result in a push response (PROTOCOL). Note the result schema has
/// no per-row error state by design: every well-formed change resolves.
#[derive(Debug, Clone, Deserialize)]
pub struct PushResult {
    pub table: String,
    pub row_id: String,
    /// `"applied"` | `"lww_won"` | `"lww_lost"`.
    pub status: String,
    pub server_seq: i64,
    /// On `lww_lost`: the winning row in pull format; else `None`.
    #[serde(default)]
    pub winner: Option<PullRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PushResponse {
    #[serde(default)]
    pub results: Vec<PushResult>,
    #[serde(default)]
    pub max_server_seq: i64,
    #[serde(default)]
    pub server_epoch: String,
    #[serde(default)]
    pub server_now_ms: i64,
    /// Additive server capabilities. Empty when an older server omits the field.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl PushResponse {
    /// Whether the server advertised the exact writing-envelope-v1 token.
    pub fn supports_writing_envelope_v1(&self) -> bool {
        supports_writing_envelope_v1(&self.capabilities)
    }

    /// Whether the server advertised the exact research-envelope-v1 token.
    pub fn supports_research_envelope_v1(&self) -> bool {
        supports_research_envelope_v1(&self.capabilities)
    }
}

/// One row returned by a pull page (PROTOCOL `GET /v1/sync/pull`). Also reused
/// as the `winner` payload in `lww_lost` push results.
#[derive(Debug, Clone, Deserialize)]
pub struct PullRow {
    pub table: String,
    pub row_id: String,
    pub server_seq: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub changed_at: i64,
    #[serde(default)]
    pub device_id: String,
    /// Row payload as a JSON object; `null` when `deleted`.
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullResponse {
    #[serde(default)]
    pub rows: Vec<PullRow>,
    #[serde(default)]
    pub next_since: i64,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub schema_tag: String,
    #[serde(default)]
    pub server_epoch: String,
    #[serde(default)]
    pub server_now_ms: i64,
    /// Additive server capabilities. Empty when an older server omits the field.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl PullResponse {
    /// Whether the server advertised the exact web-capture-v1 token.
    pub fn supports_web_capture_v1(&self) -> bool {
        supports_web_capture_v1(&self.capabilities)
    }

    /// Whether the server advertised the exact writing-envelope-v1 token.
    pub fn supports_writing_envelope_v1(&self) -> bool {
        supports_writing_envelope_v1(&self.capabilities)
    }

    /// Whether the server advertised the exact research-envelope-v1 token.
    pub fn supports_research_envelope_v1(&self) -> bool {
        supports_research_envelope_v1(&self.capabilities)
    }
}

/// Checks the web capture token exactly, case-sensitively.
pub fn supports_web_capture_v1(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|capability| capability == WEB_CAPTURE_V1_CAPABILITY)
}

/// Checks capability tokens exactly. Schema tags and successful status codes do
/// not imply writing-envelope-v1 support.
pub fn supports_writing_envelope_v1(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|capability| capability == WRITING_ENVELOPE_V1_CAPABILITY)
}

/// Checks capability tokens exactly, case-sensitively. Schema tags and
/// successful status codes do not imply research-envelope-v1 support.
pub fn supports_research_envelope_v1(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|capability| capability == RESEARCH_ENVELOPE_V1_CAPABILITY)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HealthLimits {
    #[serde(default)]
    pub max_push_bytes: i64,
    #[serde(default)]
    pub max_blob_mb: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub epoch: String,
    #[serde(default)]
    pub server_now_ms: i64,
    #[serde(default)]
    pub limits: HealthLimits,
}

/// Result of a blob HEAD probe: `true` when the server already holds the blob.
pub type BlobExists = bool;

// ---------------------------------------------------------------------------
// SyncApi trait
// ---------------------------------------------------------------------------

/// The wire contract between the client and `entropia-sync-server` (PROTOCOL).
///
/// Methods are `async`; callers drive them with `tauri::async_runtime::block_on`
/// inside the engine task (mirrors the existing OCR/transcription workers). The
/// trait is consumed generically (`<A: SyncApi>`) so the push/blob paths are
/// testable against an in-memory mock — no `dyn` / `async-trait` dependency.
///
/// Auth: every method except [`SyncApi::register`], [`SyncApi::login`] and
/// [`SyncApi::health`] attaches the device Bearer token. The `/sync/*` methods
/// also attach `X-Schema-Tag`. Ordinary [`SyncApi::push`] and [`SyncApi::pull`]
/// send only the `web-capture-v1` header; the aggregates opt in through the
/// dedicated writing-envelope-v1 or research-envelope-v1 methods.
#[allow(dead_code)]
pub trait SyncApi {
    fn register(
        &self,
        req: RegisterRequest,
    ) -> impl std::future::Future<Output = Result<RegisterResponse, SyncError>> + Send;

    fn login(
        &self,
        req: LoginRequest,
    ) -> impl std::future::Future<Output = Result<LoginResponse, SyncError>> + Send;

    fn logout(
        &self,
        token: &str,
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    fn devices(
        &self,
        token: &str,
    ) -> impl std::future::Future<Output = Result<DevicesResponse, SyncError>> + Send;

    fn revoke(
        &self,
        token: &str,
        device_id: &str,
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    fn delete_account(
        &self,
        token: &str,
        req: DeleteAccountRequest,
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    fn usage(
        &self,
        token: &str,
    ) -> impl std::future::Future<Output = Result<UsageResponse, SyncError>> + Send;

    /// Lists the plan catalog (PROTOCOL `GET /v1/plans`). No subscription gating:
    /// an expired/suspended account still gets the full catalog.
    fn list_plans(
        &self,
        token: &str,
    ) -> impl std::future::Future<Output = Result<Vec<PlanCatalogItem>, SyncError>> + Send;

    /// Requests a plan change (PROTOCOL `POST /v1/plan-change-request`). The
    /// `409 plan_request_pending` server code surfaces as a [`SyncError::Api`]
    /// with that code so the UI can show "ya tenés una solicitud en revisión".
    fn request_plan_change(
        &self,
        token: &str,
        requested_plan_id: &str,
        note: Option<&str>,
    ) -> impl std::future::Future<Output = Result<PlanChangeRequestResponse, SyncError>> + Send;

    /// Lists the user's in-app notifications (PROTOCOL `GET /v1/notifications`).
    /// `since` is an exclusive cursor by id (empty/`"0"` ⇒ from the start);
    /// `limit` is capped server-side at 100.
    fn list_notifications(
        &self,
        token: &str,
        since: Option<&str>,
        limit: Option<i64>,
    ) -> impl std::future::Future<Output = Result<Vec<NotificationItem>, SyncError>> + Send;

    /// Marks a notification as read (PROTOCOL `POST /v1/notifications/{id}/read`).
    /// Idempotent: re-marking returns 204. A 404 means the notification does not
    /// exist or belongs to another account.
    fn mark_notification_read(
        &self,
        token: &str,
        id: &str,
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    /// Deletes a notification from the user's inbox (PROTOCOL `DELETE /v1/notifications/{id}`).
    /// A 404 means the notification does not exist or belongs to another account.
    fn delete_notification(
        &self,
        token: &str,
        id: &str,
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    fn health(&self)
        -> impl std::future::Future<Output = Result<HealthResponse, SyncError>> + Send;

    fn push(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> impl std::future::Future<Output = Result<PushResponse, SyncError>> + Send;

    /// Pushes with the exact writing-envelope-v1 capability opt-in. The default
    /// fails locally so alternate implementations cannot silently issue a legacy
    /// request without the required header.
    fn push_with_writing_envelope_v1(
        &self,
        _token: &str,
        _schema_tag: &str,
        _req: PushRequest,
    ) -> impl std::future::Future<Output = Result<PushResponse, SyncError>> + Send {
        std::future::ready(Err(SyncError::Decode(
            "SyncApi implementation does not support writing-envelope-v1 opt-in".to_string(),
        )))
    }

    /// Pushes with the exact research-envelope-v1 capability opt-in. The
    /// default fails closed locally so alternate implementations cannot
    /// silently issue a legacy request without the required header.
    fn push_with_research_envelope_v1(
        &self,
        _token: &str,
        _schema_tag: &str,
        _req: PushRequest,
    ) -> impl std::future::Future<Output = Result<PushResponse, SyncError>> + Send {
        std::future::ready(Err(SyncError::Decode(
            "SyncApi implementation does not support research-envelope-v1 opt-in".to_string(),
        )))
    }

    fn pull(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> impl std::future::Future<Output = Result<PullResponse, SyncError>> + Send;

    /// Pulls with the exact writing-envelope-v1 capability opt-in. The default
    /// fails locally rather than delegating to [`SyncApi::pull`] without a header.
    fn pull_with_writing_envelope_v1(
        &self,
        _token: &str,
        _schema_tag: &str,
        _since: i64,
        _limit: i64,
    ) -> impl std::future::Future<Output = Result<PullResponse, SyncError>> + Send {
        std::future::ready(Err(SyncError::Decode(
            "SyncApi implementation does not support writing-envelope-v1 opt-in".to_string(),
        )))
    }

    /// Pulls with the exact research-envelope-v1 capability opt-in. The
    /// default fails closed rather than delegating to [`SyncApi::pull`]
    /// without a header.
    fn pull_with_research_envelope_v1(
        &self,
        _token: &str,
        _schema_tag: &str,
        _since: i64,
        _limit: i64,
    ) -> impl std::future::Future<Output = Result<PullResponse, SyncError>> + Send {
        std::future::ready(Err(SyncError::Decode(
            "SyncApi implementation does not support research-envelope-v1 opt-in".to_string(),
        )))
    }

    fn blob_head(
        &self,
        token: &str,
        sha256: &str,
    ) -> impl std::future::Future<Output = Result<BlobExists, SyncError>> + Send;

    fn blob_put(
        &self,
        token: &str,
        sha256: &str,
        bytes: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    fn blob_get(
        &self,
        token: &str,
        sha256: &str,
    ) -> impl std::future::Future<Output = Result<reqwest::Response, SyncError>> + Send;
}

// ---------------------------------------------------------------------------
// HttpSyncApi (reqwest implementation)
// ---------------------------------------------------------------------------

/// reqwest-backed [`SyncApi`]. Holds a validated base URL and a client with the
/// house-style timeouts. Blob ops override the per-request timeout with the
/// longer [`BLOB_TIMEOUT`].
pub struct HttpSyncApi {
    base_url: String,
    client: reqwest::Client,
}

impl HttpSyncApi {
    /// Builds a client for `server_url`. Re-validates the TLS rule at use-time
    /// (PROTOCOL "Transporte") so a stored URL can never be used over cleartext.
    pub fn new(server_url: &str) -> Result<Self, SyncError> {
        let base_url = validate_server_url(server_url)?;
        let client = reqwest::Client::builder()
            .user_agent("EntropIA-Desktop-Sync/1.0")
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| SyncError::Network(format!("failed to build HTTP client: {e}")))?;
        Ok(Self { base_url, client })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Maps a transport error into [`SyncError::Network`], distinguishing it from
    /// API-level errors which come from the response body.
    fn network_err(context: &str, error: reqwest::Error) -> SyncError {
        SyncError::Network(format!("{context}: {error}"))
    }

    async fn push_request(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
        capability: Option<&str>,
    ) -> Result<PushResponse, SyncError> {
        let request = self
            .client
            .post(self.url("/v1/sync/push"))
            .bearer_auth(token)
            .header(SCHEMA_TAG_HEADER, schema_tag);
        let request = match capability {
            Some(capability) => request.header(SYNC_CAPABILITIES_HEADER, capability),
            None => request,
        };
        let response = request
            .json(&req)
            .send()
            .await
            .map_err(|e| Self::network_err("push request", e))?;
        parse_json(response).await
    }

    async fn pull_request(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
        capability: Option<&str>,
    ) -> Result<PullResponse, SyncError> {
        let request = self
            .client
            .get(self.url("/v1/sync/pull"))
            .bearer_auth(token)
            .header(SCHEMA_TAG_HEADER, schema_tag);
        let request = match capability {
            Some(capability) => request.header(SYNC_CAPABILITIES_HEADER, capability),
            None => request,
        };
        let response = request
            .query(&[("since", since), ("limit", limit)])
            .send()
            .await
            .map_err(|e| Self::network_err("pull request", e))?;
        parse_json(response).await
    }
}

/// Inspects a response: on a 2xx returns it untouched; otherwise reads the body,
/// parses the `{ error: { code, message } }` envelope, and returns
/// [`SyncError::Api`]. NEVER logs the request — callers must not pass tokens here.
async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, SyncError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let code_num = status.as_u16();
    let body = response.text().await.unwrap_or_default();
    let (code, message) = match serde_json::from_str::<WireError>(&body) {
        Ok(WireError { error: Some(body) }) => (body.code, body.message),
        _ => (
            status
                .canonical_reason()
                .unwrap_or("unknown")
                .to_ascii_lowercase()
                .replace(' ', "_"),
            if body.is_empty() {
                status.to_string()
            } else {
                body
            },
        ),
    };
    Err(SyncError::Api {
        status: code_num,
        code,
        message,
    })
}

async fn parse_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, SyncError> {
    let response = ensure_success(response).await?;
    response
        .json::<T>()
        .await
        .map_err(|e| SyncError::Decode(format!("failed to parse response body: {e}")))
}

impl SyncApi for HttpSyncApi {
    async fn register(&self, req: RegisterRequest) -> Result<RegisterResponse, SyncError> {
        let response = self
            .client
            .post(self.url("/v1/auth/register"))
            .json(&req)
            .send()
            .await
            .map_err(|e| Self::network_err("register request", e))?;
        parse_json(response).await
    }

    async fn login(&self, req: LoginRequest) -> Result<LoginResponse, SyncError> {
        let response = self
            .client
            .post(self.url("/v1/auth/login"))
            .json(&req)
            .send()
            .await
            .map_err(|e| Self::network_err("login request", e))?;
        parse_json(response).await
    }

    async fn logout(&self, token: &str) -> Result<(), SyncError> {
        let response = self
            .client
            .post(self.url("/v1/auth/logout"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("logout request", e))?;
        ensure_success(response).await.map(|_| ())
    }

    async fn devices(&self, token: &str) -> Result<DevicesResponse, SyncError> {
        let response = self
            .client
            .get(self.url("/v1/devices"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("devices request", e))?;
        parse_json(response).await
    }

    async fn revoke(&self, token: &str, device_id: &str) -> Result<(), SyncError> {
        let response = self
            .client
            .delete(self.url(&format!("/v1/devices/{device_id}")))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("revoke request", e))?;
        ensure_success(response).await.map(|_| ())
    }

    async fn delete_account(
        &self,
        token: &str,
        req: DeleteAccountRequest,
    ) -> Result<(), SyncError> {
        let response = self
            .client
            .delete(self.url("/v1/account"))
            .bearer_auth(token)
            .json(&req)
            .send()
            .await
            .map_err(|e| Self::network_err("delete account request", e))?;
        ensure_success(response).await.map(|_| ())
    }

    async fn usage(&self, token: &str) -> Result<UsageResponse, SyncError> {
        let response = self
            .client
            .get(self.url("/v1/usage"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("usage request", e))?;
        parse_json(response).await
    }

    async fn list_plans(&self, token: &str) -> Result<Vec<PlanCatalogItem>, SyncError> {
        let response = self
            .client
            .get(self.url("/v1/plans"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("list plans request", e))?;
        let body: PlansResponse = parse_json(response).await?;
        Ok(body.plans)
    }

    async fn request_plan_change(
        &self,
        token: &str,
        requested_plan_id: &str,
        note: Option<&str>,
    ) -> Result<PlanChangeRequestResponse, SyncError> {
        let body = PlanChangeRequestBody {
            requested_plan_id: requested_plan_id.to_string(),
            note: note.map(str::to_string),
        };
        let response = self
            .client
            .post(self.url("/v1/plan-change-request"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| Self::network_err("plan change request", e))?;
        // The 409 `plan_request_pending` (and 404/400) come back as the structured
        // `{ error: { code, message } }` envelope, so `parse_json` → `ensure_success`
        // already maps them to `SyncError::Api { code, .. }`. Callers branch on the code.
        parse_json(response).await
    }

    async fn list_notifications(
        &self,
        token: &str,
        since: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<NotificationItem>, SyncError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(since) = since.filter(|s| !s.is_empty()) {
            query.push(("since", since.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        let response = self
            .client
            .get(self.url("/v1/notifications"))
            .bearer_auth(token)
            .query(&query)
            .send()
            .await
            .map_err(|e| Self::network_err("list notifications request", e))?;
        let body: NotificationsResponse = parse_json(response).await?;
        Ok(body.notifications)
    }

    async fn mark_notification_read(&self, token: &str, id: &str) -> Result<(), SyncError> {
        let response = self
            .client
            .post(self.url(&format!("/v1/notifications/{id}/read")))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("mark notification read request", e))?;
        ensure_success(response).await.map(|_| ())
    }

    async fn delete_notification(&self, token: &str, id: &str) -> Result<(), SyncError> {
        let response = self
            .client
            .delete(self.url(&format!("/v1/notifications/{id}")))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::network_err("delete notification request", e))?;
        ensure_success(response).await.map(|_| ())
    }

    async fn health(&self) -> Result<HealthResponse, SyncError> {
        let response = self
            .client
            .get(self.url("/v1/health"))
            .send()
            .await
            .map_err(|e| Self::network_err("health request", e))?;
        parse_json(response).await
    }

    async fn push(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.push_request(token, schema_tag, req, Some(WEB_CAPTURE_V1_CAPABILITY))
            .await
    }

    async fn push_with_writing_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.push_request(token, schema_tag, req, Some(WRITING_ENVELOPE_V1_CAPABILITY))
            .await
    }

    async fn push_with_research_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        req: PushRequest,
    ) -> Result<PushResponse, SyncError> {
        self.push_request(
            token,
            schema_tag,
            req,
            Some(RESEARCH_ENVELOPE_V1_CAPABILITY),
        )
        .await
    }

    async fn pull(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.pull_request(
            token,
            schema_tag,
            since,
            limit,
            Some(WEB_CAPTURE_V1_CAPABILITY),
        )
        .await
    }

    async fn pull_with_writing_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.pull_request(
            token,
            schema_tag,
            since,
            limit,
            Some(WRITING_ENVELOPE_V1_CAPABILITY),
        )
        .await
    }

    async fn pull_with_research_envelope_v1(
        &self,
        token: &str,
        schema_tag: &str,
        since: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        self.pull_request(
            token,
            schema_tag,
            since,
            limit,
            Some(RESEARCH_ENVELOPE_V1_CAPABILITY),
        )
        .await
    }

    async fn blob_head(&self, token: &str, sha256: &str) -> Result<BlobExists, SyncError> {
        let response = self
            .client
            .head(self.url(&format!("/v1/blobs/{sha256}")))
            .bearer_auth(token)
            .timeout(BLOB_TIMEOUT)
            .send()
            .await
            .map_err(|e| Self::network_err("blob head request", e))?;
        let status = response.status();
        if status.is_success() {
            Ok(true)
        } else if status == reqwest::StatusCode::NOT_FOUND {
            Ok(false)
        } else {
            Err(ensure_success(response).await.unwrap_err())
        }
    }

    async fn blob_put(&self, token: &str, sha256: &str, bytes: Vec<u8>) -> Result<(), SyncError> {
        let response = self
            .client
            .put(self.url(&format!("/v1/blobs/{sha256}")))
            .bearer_auth(token)
            .header("Content-Type", "application/octet-stream")
            .timeout(BLOB_TIMEOUT)
            .body(bytes)
            .send()
            .await
            .map_err(|e| Self::network_err("blob put request", e))?;
        ensure_success(response).await.map(|_| ())
    }

    async fn blob_get(&self, token: &str, sha256: &str) -> Result<reqwest::Response, SyncError> {
        let response = self
            .client
            .get(self.url(&format!("/v1/blobs/{sha256}")))
            .bearer_auth(token)
            .timeout(BLOB_TIMEOUT)
            .send()
            .await
            .map_err(|e| Self::network_err("blob get request", e))?;
        ensure_success(response).await
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::*;

    #[derive(Debug)]
    struct CapturedRequest {
        request_line: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    fn header_values<'a>(request: &'a CapturedRequest, name: &str) -> Vec<&'a str> {
        request
            .headers
            .iter()
            .filter(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }

    fn read_request(stream: &mut TcpStream) -> CapturedRequest {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set loopback read timeout");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let header_end = loop {
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position;
            }
            let read = stream.read(&mut buffer).expect("read loopback request");
            assert!(read > 0, "loopback client closed before request headers");
            bytes.extend_from_slice(&buffer[..read]);
        };

        let header_text = std::str::from_utf8(&bytes[..header_end]).expect("ASCII request headers");
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next().expect("request line").to_string();
        let headers: Vec<(String, String)> = lines
            .map(|line| {
                let (name, value) = line.split_once(':').expect("well-formed request header");
                (name.to_ascii_lowercase(), value.trim().to_string())
            })
            .collect();
        let content_length = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .map(|(_, value)| value.parse::<usize>().expect("numeric Content-Length"))
            .unwrap_or(0);
        let body_start = header_end + 4;
        while bytes.len() < body_start + content_length {
            let read = stream
                .read(&mut buffer)
                .expect("read loopback request body");
            assert!(read > 0, "loopback client closed before request body");
            bytes.extend_from_slice(&buffer[..read]);
        }

        CapturedRequest {
            request_line,
            headers,
            body: bytes[body_start..body_start + content_length].to_vec(),
        }
    }

    fn spawn_loopback_server(
        status: u16,
        response_body: &str,
    ) -> (String, Receiver<CapturedRequest>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        let address = listener.local_addr().expect("loopback server address");
        let response_body = response_body.to_string();
        let (request_tx, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept loopback request");
            let request = read_request(&mut stream);
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                500 => "Internal Server Error",
                _ => "Test Response",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write loopback response");
            stream.flush().expect("flush loopback response");
            request_tx.send(request).expect("capture loopback request");
        });
        (format!("http://{address}"), request_rx, handle)
    }

    fn finish_loopback_request(
        request_rx: Receiver<CapturedRequest>,
        handle: JoinHandle<()>,
    ) -> CapturedRequest {
        let request = request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receive loopback request");
        handle.join().expect("loopback server thread");
        request
    }

    fn sample_push_request(table: &str) -> PushRequest {
        PushRequest {
            changes: vec![PushChange {
                table: table.to_string(),
                row_id: "row-1".to_string(),
                op: "upsert".to_string(),
                changed_at: 42,
                base_seq: 7,
                payload: Some(serde_json::json!({"id":"row-1","title":"Draft"})),
            }],
        }
    }

    fn assert_auth_and_schema_headers(request: &CapturedRequest) {
        assert_eq!(
            header_values(request, "authorization"),
            vec!["Bearer test-token"]
        );
        assert_eq!(
            header_values(request, SCHEMA_TAG_HEADER),
            vec!["0023_sync_ids"]
        );
    }

    fn assert_push_body(request: &CapturedRequest, table: &str) {
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("parse captured push body");
        assert_eq!(
            body,
            serde_json::json!({
                "changes": [{
                    "table": table,
                    "row_id": "row-1",
                    "op": "upsert",
                    "changed_at": 42,
                    "base_seq": 7,
                    "payload": {"id": "row-1", "title": "Draft"}
                }]
            })
        );
    }

    #[test]
    fn validate_server_url_accepts_https_anywhere() {
        assert_eq!(
            validate_server_url("https://sync.example.com").unwrap(),
            "https://sync.example.com"
        );
        assert_eq!(
            validate_server_url("https://sync.example.com:8443/").unwrap(),
            "https://sync.example.com:8443"
        );
    }

    #[test]
    fn validate_server_url_accepts_http_only_for_loopback() {
        for url in [
            "http://127.0.0.1:8787",
            "http://localhost:8787",
            "http://[::1]:8787",
            "http://localhost",
        ] {
            assert!(validate_server_url(url).is_ok(), "should accept {url}");
        }
    }

    #[test]
    fn validate_server_url_rejects_http_to_remote_host() {
        for url in [
            "http://sync.example.com",
            "http://192.168.1.10:8787",
            "http://example.com/v1",
            "http://127.0.0.1.evil.com",
        ] {
            let err = validate_server_url(url).unwrap_err();
            assert!(
                matches!(err, SyncError::InvalidUrl(_)),
                "should reject {url}, got {err:?}"
            );
        }
    }

    #[test]
    fn validate_server_url_rejects_malformed() {
        assert!(validate_server_url("").is_err());
        assert!(validate_server_url("   ").is_err());
        assert!(validate_server_url("ftp://host").is_err());
        assert!(validate_server_url("https://").is_err());
        assert!(validate_server_url("not-a-url").is_err());
    }

    #[test]
    fn validate_server_url_strips_trailing_slash() {
        assert_eq!(
            validate_server_url("https://host/").unwrap(),
            "https://host"
        );
        assert_eq!(
            validate_server_url("https://host///").unwrap(),
            "https://host"
        );
    }

    #[test]
    fn http_sync_api_new_enforces_tls_at_use_time() {
        assert!(HttpSyncApi::new("http://remote.example.com").is_err());
        assert!(HttpSyncApi::new("http://127.0.0.1:8787").is_ok());
        assert!(HttpSyncApi::new("https://sync.example.com").is_ok());
    }

    #[test]
    fn sync_error_exposes_status_and_code() {
        let api = SyncError::Api {
            status: 426,
            code: "schema_upgrade_required".to_string(),
            message: "upgrade".to_string(),
        };
        assert_eq!(api.status(), Some(426));
        assert_eq!(api.api_code(), Some("schema_upgrade_required"));

        let net = SyncError::Network("boom".to_string());
        assert_eq!(net.status(), None);
        assert_eq!(net.api_code(), None);
    }

    #[test]
    fn wire_error_envelope_tolerates_unknown_fields() {
        let body = r#"{"error":{"code":"clock_skew","message":"reloj","extra":1},"trace":"x"}"#;
        let parsed: WireError = serde_json::from_str(body).expect("parse");
        let inner = parsed.error.expect("error body");
        assert_eq!(inner.code, "clock_skew");
        assert_eq!(inner.message, "reloj");
    }

    #[test]
    fn pull_response_tolerates_unknown_fields_and_null_payload() {
        let body = r#"{
            "rows":[{"table":"assets","row_id":"a1","server_seq":88,"deleted":true,
                     "changed_at":1,"device_id":"d1","payload":null,"future_field":"ok"}],
            "next_since":88,"has_more":false,"schema_tag":"0023_sync_ids",
            "server_epoch":"e1","server_now_ms":2,"unknown":true
        }"#;
        let parsed: PullResponse = serde_json::from_str(body).expect("parse pull");
        assert_eq!(parsed.rows.len(), 1);
        assert!(parsed.rows[0].deleted);
        assert!(parsed.rows[0].payload.is_none());
        assert_eq!(parsed.next_since, 88);
        assert!(parsed.capabilities.is_empty());
        assert!(!parsed.supports_writing_envelope_v1());
    }

    #[test]
    fn push_change_omits_payload_for_delete() {
        let change = PushChange {
            table: "items".to_string(),
            row_id: "i1".to_string(),
            op: "delete".to_string(),
            changed_at: 10,
            base_seq: 3,
            payload: None,
        };
        let json = serde_json::to_value(&change).expect("serialize");
        assert!(
            json.get("payload").is_none(),
            "delete change must omit payload"
        );
        assert_eq!(json["op"], "delete");
    }

    #[test]
    fn push_response_tolerates_missing_winner_and_unknown_fields() {
        let body = r#"{
            "results":[{"table":"items","row_id":"i1","status":"applied","server_seq":5}],
            "max_server_seq":5,"server_epoch":"e1","server_now_ms":9,"unknown":42
        }"#;
        let parsed: PushResponse = serde_json::from_str(body).expect("parse push");
        assert_eq!(parsed.results.len(), 1);
        assert!(parsed.results[0].winner.is_none());
        assert_eq!(parsed.results[0].status, "applied");
        assert!(parsed.capabilities.is_empty());
        assert!(!parsed.supports_writing_envelope_v1());
    }

    #[test]
    fn usage_response_parses_old_server_without_notification_fields() {
        // Backward compat: an older server that only sends the original fields must
        // still deserialize, with the new fields defaulting (None / 0).
        let body = r#"{
            "rows":7,"blobs_count":2,"blobs_bytes":1024,"quota_bytes":104857600,
            "plan_name":"Free"
        }"#;
        let parsed: UsageResponse = serde_json::from_str(body).expect("parse old usage");
        assert_eq!(parsed.rows, 7);
        assert_eq!(parsed.plan_name.as_deref(), Some("Free"));
        assert_eq!(parsed.expires_at, None);
        assert_eq!(parsed.unread_notifications, 0);
        assert_eq!(parsed.pending_plan_request, None);
    }

    #[test]
    fn usage_response_parses_new_server_with_all_fields() {
        let body = r#"{
            "rows":7,"blobs_count":2,"blobs_bytes":1024,"quota_bytes":10737418240,
            "plan_name":"10 GB","expires_at":1760000000000,"unread_notifications":3,
            "pending_plan_request":"50 GB","future_field":"ignored"
        }"#;
        let parsed: UsageResponse = serde_json::from_str(body).expect("parse new usage");
        assert_eq!(parsed.expires_at, Some(1760000000000));
        assert_eq!(parsed.unread_notifications, 3);
        assert_eq!(parsed.pending_plan_request.as_deref(), Some("50 GB"));
        assert_eq!(parsed.plan_name.as_deref(), Some("10 GB"));
    }

    #[test]
    fn usage_response_handles_explicit_null_optionals() {
        // The server serializes None as `null` (not omitted) — must parse to None.
        let body = r#"{
            "rows":0,"blobs_count":0,"blobs_bytes":0,"quota_bytes":0,
            "plan_name":null,"expires_at":null,"unread_notifications":0,
            "pending_plan_request":null
        }"#;
        let parsed: UsageResponse = serde_json::from_str(body).expect("parse null usage");
        assert_eq!(parsed.plan_name, None);
        assert_eq!(parsed.expires_at, None);
        assert_eq!(parsed.pending_plan_request, None);
    }

    #[test]
    fn plans_response_parses_catalog_with_null_description() {
        let body = r#"{"plans":[
            {"id":"p0","name":"Free","quota_bytes":104857600,"price_cents":0,
             "currency":"USD","period":"none","description":"Plan gratuito","is_current":true},
            {"id":"p1","name":"5 GB","quota_bytes":5368709120,"price_cents":1000,
             "currency":"USD","period":"monthly","description":null,"is_current":false,
             "future_field":1}
        ]}"#;
        let parsed: PlansResponse = serde_json::from_str(body).expect("parse plans");
        assert_eq!(parsed.plans.len(), 2);
        assert!(parsed.plans[0].is_current);
        assert_eq!(
            parsed.plans[0].description.as_deref(),
            Some("Plan gratuito")
        );
        assert_eq!(parsed.plans[1].description, None);
        assert!(!parsed.plans[1].is_current);
        assert_eq!(parsed.plans[1].price_cents, 1000);
    }

    #[test]
    fn notification_item_parses_without_category_and_with_null_read_at() {
        // The real server inbox shape: no `category`, `read_at` null when unread.
        let body = r#"{
            "id":"n1","kind":"subscription_reminder","severity":"warning",
            "title":"Vence en 7 días","body":"Renová a tiempo","created_at":1760000000000,
            "read_at":null,"unknown":true
        }"#;
        let parsed: NotificationItem = serde_json::from_str(body).expect("parse notif");
        assert_eq!(parsed.kind, "subscription_reminder");
        assert_eq!(parsed.severity, "warning");
        assert_eq!(parsed.category, ""); // defaulted — server doesn't send it today
        assert_eq!(parsed.read_at, None);
    }

    #[test]
    fn plan_change_request_response_parses_with_null_current_plan() {
        let body = r#"{
            "id":"r1","current_plan_id":null,"requested_plan_id":"p2",
            "note":"quiero más espacio","status":"pending","created_at":1760000000000
        }"#;
        let parsed: PlanChangeRequestResponse =
            serde_json::from_str(body).expect("parse plan change");
        assert_eq!(parsed.id, "r1");
        assert_eq!(parsed.current_plan_id, None);
        assert_eq!(parsed.requested_plan_id, "p2");
        assert_eq!(parsed.status, "pending");
    }

    #[test]
    fn plan_change_request_body_omits_note_when_absent() {
        let body = PlanChangeRequestBody {
            requested_plan_id: "p2".to_string(),
            note: None,
        };
        let json = serde_json::to_value(&body).expect("serialize");
        assert!(json.get("note").is_none(), "absent note must be omitted");
        assert_eq!(json["requested_plan_id"], "p2");
    }

    #[test]
    fn writing_capability_support_requires_exact_case_sensitive_token() {
        assert!(supports_writing_envelope_v1(&[
            WRITING_ENVELOPE_V1_CAPABILITY.to_string()
        ]));
        for unsupported in [
            Vec::new(),
            vec!["Writing-envelope-v1".to_string()],
            vec!["writing-envelope-v1-extra".to_string()],
            vec!["writing-envelope-v1 ".to_string()],
        ] {
            assert!(
                !supports_writing_envelope_v1(&unsupported),
                "unexpected support for {unsupported:?}"
            );
        }
    }

    #[tokio::test]
    async fn ordinary_push_sends_only_the_web_capture_capability_and_preserves_request() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"results":[],"max_server_seq":7,"server_epoch":"epoch","server_now_ms":43}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let response = api
            .push("test-token", "0023_sync_ids", sample_push_request("items"))
            .await
            .expect("ordinary push");
        let request = finish_loopback_request(request_rx, handle);

        assert!(!response.supports_writing_envelope_v1());
        assert_eq!(request.request_line, "POST /v1/sync/push HTTP/1.1");
        assert_auth_and_schema_headers(&request);
        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![WEB_CAPTURE_V1_CAPABILITY]
        );
        assert_push_body(&request, "items");
    }

    #[tokio::test]
    async fn ordinary_pull_sends_only_the_web_capture_capability_and_preserves_query() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"rows":[],"next_since":41,"has_more":false,"schema_tag":"0023_sync_ids","server_epoch":"epoch","server_now_ms":43,"capabilities":[]}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let response = api
            .pull("test-token", "0023_sync_ids", 41, 17)
            .await
            .expect("ordinary pull");
        let request = finish_loopback_request(request_rx, handle);

        assert!(response.capabilities.is_empty());
        assert!(!response.supports_writing_envelope_v1());
        assert_eq!(
            request.request_line,
            "GET /v1/sync/pull?since=41&limit=17 HTTP/1.1"
        );
        assert_auth_and_schema_headers(&request);
        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![WEB_CAPTURE_V1_CAPABILITY]
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn opted_in_push_sends_exact_capability_and_preserves_request() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"results":[],"max_server_seq":8,"server_epoch":"epoch","server_now_ms":43,"capabilities":["writing-envelope-v1"]}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let response = api
            .push_with_writing_envelope_v1(
                "test-token",
                "0023_sync_ids",
                sample_push_request("writing_envelopes"),
            )
            .await
            .expect("capability push");
        let request = finish_loopback_request(request_rx, handle);

        assert!(response.supports_writing_envelope_v1());
        assert_eq!(request.request_line, "POST /v1/sync/push HTTP/1.1");
        assert_auth_and_schema_headers(&request);
        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![WRITING_ENVELOPE_V1_CAPABILITY]
        );
        assert_push_body(&request, "writing_envelopes");
    }

    #[tokio::test]
    async fn opted_in_pull_sends_exact_capability_and_preserves_query() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"rows":[],"next_since":0,"has_more":false,"schema_tag":"0023_sync_ids","server_epoch":"epoch","server_now_ms":43,"capabilities":["writing-envelope-v1"]}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let response = api
            .pull_with_writing_envelope_v1("test-token", "0023_sync_ids", 0, 500)
            .await
            .expect("capability pull");
        let request = finish_loopback_request(request_rx, handle);

        assert!(response.supports_writing_envelope_v1());
        assert_eq!(
            request.request_line,
            "GET /v1/sync/pull?since=0&limit=500 HTTP/1.1"
        );
        assert_auth_and_schema_headers(&request);
        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![WRITING_ENVELOPE_V1_CAPABILITY]
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn opted_in_push_propagates_structured_api_error() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            400,
            r#"{"error":{"code":"bad_request","message":"push rejected"}}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let error = api
            .push_with_writing_envelope_v1(
                "test-token",
                "0023_sync_ids",
                sample_push_request("writing_envelopes"),
            )
            .await
            .expect_err("push must propagate error");
        let _request = finish_loopback_request(request_rx, handle);

        assert_eq!(error.status(), Some(400));
        assert_eq!(error.api_code(), Some("bad_request"));
    }

    #[tokio::test]
    async fn opted_in_pull_propagates_structured_api_error() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            400,
            r#"{"error":{"code":"bad_request","message":"pull rejected"}}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let error = api
            .pull_with_writing_envelope_v1("test-token", "0023_sync_ids", 0, 500)
            .await
            .expect_err("pull must propagate error");
        let _request = finish_loopback_request(request_rx, handle);

        assert_eq!(error.status(), Some(400));
        assert_eq!(error.api_code(), Some("bad_request"));
    }

    #[test]
    fn research_capability_support_requires_exact_case_sensitive_token() {
        assert!(supports_research_envelope_v1(&[
            RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()
        ]));
        assert!(supports_research_envelope_v1(&[
            WRITING_ENVELOPE_V1_CAPABILITY.to_string(),
            RESEARCH_ENVELOPE_V1_CAPABILITY.to_string()
        ]));
        for unsupported in [
            Vec::new(),
            vec!["Research-envelope-v1".to_string()],
            vec!["RESEARCH-ENVELOPE-V1".to_string()],
            vec!["research-envelope-v1-extra".to_string()],
            vec!["research-envelope-v1 ".to_string()],
        ] {
            assert!(
                !supports_research_envelope_v1(&unsupported),
                "unexpected support for {unsupported:?}"
            );
        }
    }

    #[test]
    fn research_capability_token_matches_the_negotiation_constant() {
        assert_eq!(
            RESEARCH_ENVELOPE_V1_CAPABILITY,
            crate::sync::research_capture::RESEARCH_CAPABILITY
        );
    }

    #[test]
    fn push_and_pull_responses_expose_the_research_capability() {
        let push: PushResponse = serde_json::from_str(
            r#"{"results":[],"max_server_seq":1,"server_epoch":"e","server_now_ms":2,
                "capabilities":["research-envelope-v1"]}"#,
        )
        .expect("parse push");
        assert!(push.supports_research_envelope_v1());
        assert!(!push.supports_writing_envelope_v1());

        let pull: PullResponse = serde_json::from_str(
            r#"{"rows":[],"next_since":1,"has_more":false,"schema_tag":"s","server_epoch":"e",
                "server_now_ms":2,"capabilities":["research-envelope-v1"]}"#,
        )
        .expect("parse pull");
        assert!(pull.supports_research_envelope_v1());
        assert!(!pull.supports_writing_envelope_v1());
    }

    #[tokio::test]
    async fn research_opted_in_push_sends_exactly_the_research_capability() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"results":[],"max_server_seq":8,"server_epoch":"epoch","server_now_ms":43,"capabilities":["research-envelope-v1"]}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let response = api
            .push_with_research_envelope_v1(
                "test-token",
                "0023_sync_ids",
                sample_push_request("research_envelopes"),
            )
            .await
            .expect("research capability push");
        let request = finish_loopback_request(request_rx, handle);

        assert!(response.supports_research_envelope_v1());
        assert!(!response.supports_writing_envelope_v1());
        assert_eq!(request.request_line, "POST /v1/sync/push HTTP/1.1");
        assert_auth_and_schema_headers(&request);
        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![RESEARCH_ENVELOPE_V1_CAPABILITY]
        );
        assert_push_body(&request, "research_envelopes");
    }

    #[tokio::test]
    async fn research_opted_in_pull_sends_exactly_the_research_capability() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"rows":[],"next_since":0,"has_more":false,"schema_tag":"0023_sync_ids","server_epoch":"epoch","server_now_ms":43,"capabilities":["research-envelope-v1"]}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let response = api
            .pull_with_research_envelope_v1("test-token", "0023_sync_ids", 0, 500)
            .await
            .expect("research capability pull");
        let request = finish_loopback_request(request_rx, handle);

        assert!(response.supports_research_envelope_v1());
        assert_eq!(
            request.request_line,
            "GET /v1/sync/pull?since=0&limit=500 HTTP/1.1"
        );
        assert_auth_and_schema_headers(&request);
        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![RESEARCH_ENVELOPE_V1_CAPABILITY]
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn writing_opt_in_never_sends_the_research_capability() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            200,
            r#"{"results":[],"max_server_seq":8,"server_epoch":"epoch","server_now_ms":43,"capabilities":["writing-envelope-v1"]}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        api.push_with_writing_envelope_v1(
            "test-token",
            "0023_sync_ids",
            sample_push_request("writing_envelopes"),
        )
        .await
        .expect("writing capability push");
        let request = finish_loopback_request(request_rx, handle);

        assert_eq!(
            header_values(&request, SYNC_CAPABILITIES_HEADER),
            vec![WRITING_ENVELOPE_V1_CAPABILITY],
            "the writing opt-in sends only its own token"
        );
    }

    #[tokio::test]
    async fn research_opted_in_push_propagates_structured_api_error() {
        let (server_url, request_rx, handle) = spawn_loopback_server(
            400,
            r#"{"error":{"code":"bad_request","message":"push rejected"}}"#,
        );
        let api = HttpSyncApi::new(&server_url).expect("HTTP client");

        let error = api
            .push_with_research_envelope_v1(
                "test-token",
                "0023_sync_ids",
                sample_push_request("research_envelopes"),
            )
            .await
            .expect_err("push must propagate error");
        let _request = finish_loopback_request(request_rx, handle);

        assert_eq!(error.status(), Some(400));
        assert_eq!(error.api_code(), Some("bad_request"));
    }

    #[tokio::test]
    async fn research_capability_defaults_fail_closed_and_leave_other_paths_untouched() {
        use crate::sync::test_support::MockSyncApi;

        let api = MockSyncApi::default();
        let push_error = api
            .push_with_research_envelope_v1("tok", "0023_sync_ids", sample_push_request("items"))
            .await
            .expect_err("research push default must fail closed");
        assert!(matches!(push_error, SyncError::Decode(_)));
        let pull_error = api
            .pull_with_research_envelope_v1("tok", "0023_sync_ids", 0, 10)
            .await
            .expect_err("research pull default must fail closed");
        assert!(matches!(pull_error, SyncError::Decode(_)));

        // Ordinary and writing requests on the same double are untouched.
        api.push("tok", "0023_sync_ids", sample_push_request("items"))
            .await
            .expect("ordinary push");
        api.pull("tok", "0023_sync_ids", 0, 10)
            .await
            .expect("ordinary pull");
        api.push_with_writing_envelope_v1("tok", "0023_sync_ids", sample_push_request("items"))
            .await
            .expect("writing push");
        assert_eq!(api.pushed_count(), 2);
    }
}
