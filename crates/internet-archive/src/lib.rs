//! Internet Archive client for the EntropIA connector: search, item metadata
//! with PDF availability, and identifier resolution, behind a strict fetch
//! policy.
//!
//! Only `https://archive.org` and `https://*.archive.org` are reachable in
//! production, every resolved address is checked against the public-internet
//! predicates (mirroring `url_policy.rs` in the desktop app), the validated
//! address is pinned per request so DNS rebinding cannot swap it, redirects
//! are followed manually with re-validation, and response bodies are capped.

use std::time::Duration;

pub mod client;
pub mod download;
pub mod error;
pub mod item;
pub mod job;
pub mod name;
pub mod policy;
pub mod prepare;
pub mod provenance;
pub mod resolve;
pub mod search;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod verify;

pub use client::IaClient;
pub use download::{
    DownloadLimits, RetryPolicy, ABSOLUTE_MAX_DOWNLOAD_BYTES, DEFAULT_MAX_DOWNLOAD_BYTES,
};
pub use error::IaError;
pub use item::{
    parse_item_value, Availability, FileSource, ItemDetails, PdfFile, RestrictedReason,
    MAX_PDFS_PER_ITEM,
};
pub use name::display_file_name;
pub use policy::{is_public_ip, FetchPolicy};
pub use provenance::{
    build_provenance, parse_provenance, ItemSnapshot, ProvenanceError, ProvenanceRecordParts,
    MAX_PROVENANCE_BYTES, PROVENANCE_KEY,
};
pub use resolve::resolve_identifier;
pub use search::{Endpoints, SearchItem, SearchPage, SearchQuery};
pub use verify::{
    verify_staged_pdf, ExpectedRemote, PageLimits, VerifiedPdf, ABSOLUTE_MAX_PAGES,
    DEFAULT_MAX_PAGES,
};

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
