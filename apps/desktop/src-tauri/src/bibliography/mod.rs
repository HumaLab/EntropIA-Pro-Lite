//! Persistent bibliography catalog foundations.
//!
//! This module owns the E1b catalog persistence seam through collections, tags,
//! attachments, memberships, explicit tombstones and durable reconciliation
//! state and the confirmed local-personal catalog read projection, plus the
//! E2b processing arm (`processing`) that runs admitted library syncs behind
//! the batch queue. Selectors and file opening belong to later slices.

pub mod attachment;
pub mod chunks;
pub mod commands;
pub mod compose;
pub mod detail;
pub mod eval;
pub mod generation;
pub mod html_text;
pub mod ingest;
pub mod processing;
pub mod profile;
pub mod reconciliation;
pub mod repository;
pub mod reprocess;
pub mod retrieval;
pub mod selective_ocr;
pub mod validate;
pub mod vector_index;
pub mod web_upload;
pub mod work_view;

pub use detail::*;
pub use reconciliation::*;
