//! Persistent bibliography catalog foundations.
//!
//! This module owns the E1b catalog persistence seam through collections, tags,
//! attachments, memberships and explicit tombstones. Reconciliation, catalog
//! reads, selectors and file opening belong to later slices.

pub mod repository;
