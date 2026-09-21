//! Persistent bibliography catalog foundations.
//!
//! This module owns the E1b catalog persistence seam through collections, tags,
//! attachments, memberships, explicit tombstones and durable reconciliation
//! state. Catalog reads, selectors and file opening belong to later slices.

pub mod reconciliation;
pub mod repository;

pub use reconciliation::*;
