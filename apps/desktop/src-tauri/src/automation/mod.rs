//! The agent endpoint: lets an external assistant (Claude Code, Codex, OpenCode,
//! Pi) operate this installed app through MCP. See `integrations/agents/`.
//!
//! - [`access`] — paired clients, their tokens and scopes.
//! - [`approvals`] — the owner's approval of paid work an assistant asked for.
//! - [`audit`] — the local trail of what an assistant did (S9).
//! - [`commands`] — the Tauri commands the Asistentes panel calls (D11).
//! - [`connector`] — where the bundled assistant connector is installed (D15).
//! - [`corpus_tools`] — the corpus edit tools' adapter over the corpus services.
//! - [`jobs`] — the processing tools' adapter over the batch services.
//! - [`mcp`] — what the endpoint offers over MCP (one tool per intent).
//! - [`proof`] — the instance key and HMAC that keep a token from reaching a
//!   process that is not this app (D12).
//! - [`read`] — the read-only archive connection and the queries behind the tools.
//! - [`server`] — the loopback HTTP endpoint, its request guard and lifecycle.

pub mod access;
pub mod approvals;
pub mod audit;
pub mod commands;
pub mod connector;
pub mod corpus_tools;
pub mod jobs;
pub mod mcp;
#[cfg(windows)]
mod private_acl;
pub mod proof;
pub mod read;
pub mod server;

#[cfg(test)]
pub(crate) mod mcp_tests;
#[cfg(test)]
pub(crate) mod test_archive;
