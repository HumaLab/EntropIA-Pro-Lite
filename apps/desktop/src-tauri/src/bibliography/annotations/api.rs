//! The Zotero Web API calls the annotation sync needs, behind a trait so the
//! engine can be tested against an in-memory Zotero.

use std::collections::BTreeMap;

use super::wire::Data;

/// A versioned answer: the value and the response's `Last-Modified-Version`.
#[derive(Debug, Clone, PartialEq)]
pub struct Versioned<T> {
    pub value: T,
    pub library_version: i64,
}

/// One item as the server holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerItem {
    pub version: i64,
    pub data: Data,
}

/// The per-key result of a POST (HTTP 200 with `successful` / `unchanged` /
/// `failed`).
#[derive(Debug, Clone, PartialEq)]
pub enum CreateResult {
    Created { version: i64 },
    /// `failed` with code 412: the key already exists.
    Exists,
    /// The answer has no entry for this object: its outcome is unknown.
    NoAnswer,
    /// `failed` with another code; `parent` when Zotero says the parent item
    /// was not found.
    Failed { code: u16, message: String, parent_missing: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteResult {
    /// 204 with the new `Last-Modified-Version`.
    Done { version: i64 },
    /// 404 (DELETE only).
    NotFound,
    /// 412.
    Stale,
    /// Another 4xx.
    Rejected { code: u16, message: String },
}

/// Why a call produced no usable answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// Nothing answered, or the answer was cut or unreadable. `after_send`
    /// tells whether the body may have reached the server.
    Unreachable { after_send: bool },
    /// 5xx, or a success whose body cannot be read: a write may have
    /// landed. 503 may carry `Retry-After` (seconds).
    Server { code: u16, retry_after: Option<u64> },
    /// 401 or 403.
    Credentials,
    /// 409 or 429 with `Retry-After` (seconds): the request was refused.
    RetryAfter { seconds: u64 },
}

/// A `Backoff` header seen on any response, success included.
pub type Backoff = Option<u64>;

pub trait AnnotationApi {
    /// `GET items?itemType=annotation&since=V&format=versions&includeTrashed=1`.
    fn versions_since(&self, since: i64) -> Result<Versioned<BTreeMap<String, i64>>, ApiError>;
    /// `GET items?itemKey=…&includeTrashed=1`, at most 50 keys.
    fn items(&self, keys: &[String]) -> Result<Versioned<Vec<ServerItem>>, ApiError>;
    /// `GET deleted?since=V`, the item keys.
    fn deleted_since(&self, since: i64) -> Result<Versioned<Vec<String>>, ApiError>;
    /// `GET items/{key}?includeTrashed=1`; `None` on 404.
    fn item(&self, key: &str) -> Result<Option<ServerItem>, ApiError>;
    /// `POST items`, at most 50 objects; results in request order.
    fn create(&self, items: &[Data]) -> Result<Vec<CreateResult>, ApiError>;
    /// `PATCH items/{key}` with `If-Unmodified-Since-Version`.
    fn patch(&self, key: &str, version: i64, fields: &Data) -> Result<WriteResult, ApiError>;
    /// `DELETE items/{key}` with `If-Unmodified-Since-Version`.
    fn delete(&self, key: &str, version: i64) -> Result<WriteResult, ApiError>;
    /// The last `Backoff` header seen, cleared by reading it.
    fn take_backoff(&self) -> Backoff;
}
