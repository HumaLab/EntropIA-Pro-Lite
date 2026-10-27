//! Zotero annotations of the Biblioteca (`docs/zotero-annotations-plan.md`).
//!
//! `model` and `machine` are the pure sync state machine: rows, invariants,
//! local mutations, the pull merge, push outcomes, uncertain-outcome
//! reconciliation and conflict resolution. `wire` knows the Zotero `data`
//! object. I/O lives in the repository and transport modules that call them.

pub mod api;
pub mod commands;
pub mod engine;
pub mod http;
pub mod machine;
pub mod repo;
#[cfg(test)]
pub(crate) mod repo_test_support;
pub mod model;
pub mod wire;

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod machine_tests;

pub use machine::*;
pub use model::*;
