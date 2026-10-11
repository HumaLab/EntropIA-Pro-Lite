pub mod authorizer;
// Canonical schema dump for the Phase 4 migration equivalence tests. Kept in
// the production graph (not `#[cfg(test)]`) so 4.2b can reuse it directly;
// until then only tests reach it, hence the allow.
#[allow(dead_code)]
pub(crate) mod canonical_dump;
pub mod commands;
pub mod compact;
pub mod migration_allowlist;
pub mod open;
pub mod sql_split;
pub mod state;
pub mod util;
