//! Strict HTTP fetch policy for the Internet Archive connector.
//!
//! Only `https://archive.org` and `https://*.archive.org` are reachable in
//! production, every resolved address is checked against the public-internet
//! predicates (mirroring `url_policy.rs` in the desktop app), the validated
//! address is pinned per request so DNS rebinding cannot swap it, redirects
//! are followed manually with re-validation, and response bodies are capped.

use std::time::Duration;

pub mod client;
pub mod error;
pub mod item;
pub mod policy;
pub mod resolve;
pub mod search;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use client::IaClient;
pub use error::IaError;
pub use item::{
    Availability, FileSource, ItemDetails, PdfFile, RestrictedReason, MAX_PDFS_PER_ITEM,
};
pub use policy::{is_public_ip, FetchPolicy};
pub use resolve::resolve_identifier;
pub use search::{Endpoints, SearchItem, SearchPage, SearchQuery};

/// Cap for item metadata payloads.
pub const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
/// Cap for search payloads.
pub const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;
/// How long a single redirect chain may grow (follows, not requests).
pub const MAX_REDIRECTS: usize = 5;
/// TCP connect timeout, kept separate from the overall request timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall request timeout (connect + response body).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Descriptive agent so archive.org operators can identify the connector.
pub const USER_AGENT: &str = concat!(
    "EntropIA/",
    env!("CARGO_PKG_VERSION"),
    " (+internet-archive connector)"
);
