//! Copying a saved web source to Zotero (Navegador phase 5).
//!
//! A copy is one durable row ([`store`]) that says "put this source, or this PDF
//! capture, in that Zotero library". The row is the queue: when Zotero is not
//! running the row waits, and a drain ([`run`]) works through the waiting rows
//! the moment Zotero answers. Nothing here runs on its own: a copy starts when
//! the person asks, and a drain starts when the app asks (view open, button).
//!
//! Why this is not the bibliographic ingest tray (`bibliographic_ingest_operations`):
//! its `kind` CHECK cannot take a new kind without a migration, its rows hang
//! off `zotero_libraries` (the semantic catalog, which a Lite install may never
//! populate), and its live transport only targets one hard-coded test group.
//! This queue is device-local and created at runtime like `sync_web_pending_blobs`.
//!
//! The write path is Zotero's own connector (`/connector/saveItems`,
//! `updateSession`, `saveAttachment`): it needs Zotero desktop running and no
//! API key. It can only create. The local API is read-only and no key is stored
//! in the app, so an item that already exists is found and linked, never
//! duplicated and never edited; [`plan`] says what would differ.

pub mod launch;
pub mod plan;
pub mod port;
pub mod run;
pub mod store;
