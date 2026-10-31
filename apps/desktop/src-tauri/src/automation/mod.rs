//! The agent endpoint: lets an external assistant (Claude Code, Codex, OpenCode,
//! Pi) operate this installed app through MCP. See `integrations/agents/`.
//!
//! - [`access`] — paired clients, their tokens and scopes.

pub mod access;
