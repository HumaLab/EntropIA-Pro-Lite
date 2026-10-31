//! The agent endpoint: lets an external assistant (Claude Code, Codex, OpenCode,
//! Pi) operate this installed app through MCP. See `integrations/agents/`.
//!
//! - [`access`] — paired clients, their tokens and scopes.
//! - [`audit`] — the local trail of what an assistant did (S9).
//! - [`mcp`] — what the endpoint offers over MCP (one tool per intent).
//! - [`read`] — the read-only archive connection and the queries behind the tools.
//! - [`server`] — the loopback HTTP endpoint, its request guard and lifecycle.

pub mod access;
pub mod audit;
pub mod mcp;
pub mod read;
pub mod server;

#[cfg(test)]
pub(crate) mod mcp_tests;
#[cfg(test)]
pub(crate) mod test_archive;
