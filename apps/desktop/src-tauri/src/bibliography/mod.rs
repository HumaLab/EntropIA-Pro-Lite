//! Persistent bibliography catalog foundations.
//!
//! This module owns the E1b catalog persistence seam through collections, tags,
//! attachments, memberships, explicit tombstones and durable reconciliation
//! state and the confirmed local-personal catalog read projection. Selectors and
//! file opening belong to later slices.

pub mod detail;
pub mod reconciliation;
pub mod repository;

pub use detail::*;
pub use reconciliation::*;
