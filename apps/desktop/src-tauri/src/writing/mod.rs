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
//! None of the `writing_*` tables are in `sync::capture::SYNCED_TABLES`, so
//! they are invisible to the sync engine — the same treatment `rag_chunks` and
//! the `processing_*` family already get. This is a deliberate choice, not an
//! oversight: adding them means bumping `TRIGGERS_VERSION`, which forces a
//! DROP-all-then-create of all 48 existing capture triggers for every user,
//! including those who never open this section. Revisit once the MVP runs end
//! to end and it is clear which of these tables are worth replicating — the
//! citation projections are regenerable from the canonical JSON and the
//! suggestions are bound to one revision, so neither is an obvious candidate.
//! Until then a manuscript lives on the machine that wrote it.

pub mod commands;
pub mod journal;
pub mod recovery;
pub mod repository;
pub mod versions;
