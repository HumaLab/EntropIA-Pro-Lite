//! Persistent bibliography catalog foundations.
//!
//! This module deliberately stops at connection/library/item persistence. The
//! later catalog slices own collections, tags, attachments, reconciliation and
//! read/selector seams.

pub mod repository;
