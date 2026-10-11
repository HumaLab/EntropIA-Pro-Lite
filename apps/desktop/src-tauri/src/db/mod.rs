pub mod authorizer;
// Canonical schema dump for the Phase 4 migration equivalence tests; its only
// consumer is db::migrations' test module, so it is compiled for tests alone
// instead of shipping a dead_code allow in the release graph.
#[cfg(test)]
pub(crate) mod canonical_dump;
pub mod commands;
pub mod compact;
pub mod migration_allowlist;
// Rust port of the renderer migration runner, wired into `setup` at startup
// (Phase 4.2c). `repairs` wraps it with the idempotent schema repairs (4.3b).
pub(crate) mod migrations;
pub mod open;
pub(crate) mod repairs;
pub mod sql_split;
pub mod state;
pub mod util;
