//! The agent endpoint: lets an external assistant (Claude Code, Codex, OpenCode,
//! Pi) operate this installed app through MCP. See `integrations/agents/`.
//!
//! - [`access`] — paired clients, their tokens and scopes.
//! - [`mcp`] — what the endpoint offers over MCP (one tool per intent).
//! - [`server`] — the loopback HTTP endpoint, its request guard and lifecycle.

pub mod access;
pub mod mcp;
pub mod server;
