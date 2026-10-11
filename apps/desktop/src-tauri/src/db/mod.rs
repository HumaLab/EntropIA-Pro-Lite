pub mod authorizer;
// Canonical schema dump for the Phase 4 migration equivalence tests. Kept in
// the production graph (not `#[cfg(test)]`) so db::migrations can reuse it;
// 4.2c wires the runner into setup and removes the allow.
#[allow(dead_code)]
pub(crate) mod canonical_dump;
pub mod commands;
pub mod compact;
pub mod migration_allowlist;
// Rust port of the renderer migration runner. Exercised by its own tests but
// not wired into `setup` yet; 4.2c wires it and removes the allow.
#[allow(dead_code)]
pub(crate) mod migrations;
pub mod open;
pub mod sql_split;
pub mod state;
pub mod util;
