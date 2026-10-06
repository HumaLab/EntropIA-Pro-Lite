//! Writing workspace: the academic manuscript and its provenance
//! (plan-editor.md). One module owns the canonical content, its revision, the
//! citation projections and the provenance log; the Svelte UI edits and reads,
//! but never decides what is durable.
//!
//! Persistence lives here rather than in `packages/store` for one measured
//! reason. §8.5 requires a save to carry the revision it believes it is
//! replacing, and to be rejected atomically if the document has moved on. The
//! generic `db_execute_transaction` IPC cannot express that: it discards every
//! statement's affected-row count, so a conditional
//! `UPDATE ... WHERE revision = ?` that matches nothing commits as a silent
//! no-op. The batch queue already solved the same problem with a conditional
//! update plus a row-count check (`processing::repository`), and this module
//! reuses that shape instead of widening the SQL gateway every other feature
//! shares.
//!
//! The schema itself still belongs to `packages/store` — migration
//! `0035_writing_workspace`, applied by the frontend runner — exactly as the
//! `processing_*` tables do. The backend never runs DDL; it verifies presence.
//!
//! # Sync
//!
//! The `writing_*` tables remain outside generic row capture. The inactive sync
//! adapters can snapshot, receive, and preserve a deterministic conflict copy
//! through caller-owned savepoints, and the offline file layer (`sync_files`)
//! can prove and install the referenced image/crop files, but no capture,
//! transport, pull, or UI path invokes them yet. Receive also requires a caller assertion from the future
//! verified attachment-install stage; the database adapters never claim files
//! are ready.

pub mod agent;
pub mod agent_actions;
pub mod agent_prompt;
pub mod commands;
pub mod csl;
pub mod journal;
pub mod publish;
pub mod recovery;
pub mod repository;
pub mod retrieval;
pub(crate) mod sync_capture;
#[cfg(test)]
mod sync_capture_atomic_tests;
#[cfg(test)]
mod sync_capture_tests;
pub(crate) mod sync_conflict;
#[cfg(test)]
mod sync_conflict_tests;
pub(crate) mod sync_envelope;
#[cfg(test)]
mod sync_envelope_tests;
pub(crate) mod sync_files;
#[cfg(test)]
mod sync_files_tests;
pub(crate) mod sync_receive;
#[cfg(test)]
mod sync_receive_tests;
pub(crate) mod sync_shared;
pub(crate) mod sync_transport;
#[cfg(test)]
mod sync_transport_tests;
pub mod versions;
pub mod zotero;
