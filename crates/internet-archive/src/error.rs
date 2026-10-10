//! Fetch failure modes for the Internet Archive connector.
//!
//! Remote input never panics; it maps to one of these variants.

use std::time::Duration;

/// Fetch failure modes. Remote input never panics; it maps to one of these.
#[derive(Debug, thiserror::Error)]
pub enum IaError {
    /// The URL or a redirect target violates the fetch policy.
    #[error("blocked: {reason}")]
    Blocked { reason: String },
    /// More than [`crate::MAX_REDIRECTS`] redirect hops were requested.
    #[error("too many redirects")]
    TooManyRedirects,
    /// The body exceeds the caller-supplied cap.
    #[error("response body exceeds limit of {limit} bytes")]
    TooLarge { limit: usize },
    /// A non-success HTTP status. `retry_after` is set for 429 with seconds.
    #[error("http error: {status}")]
    Http {
        status: u16,
        retry_after: Option<Duration>,
    },
    /// The search query is invalid or would page past the allowed window.
    #[error("invalid query: {reason}")]
    InvalidQuery { reason: String },
    /// Transport, DNS or timeout failure.
    #[error("network error: {0}")]
    Network(String),
    /// The body was received within the cap but is not the expected JSON.
    #[error("decode error: {0}")]
    Decode(String),
}
