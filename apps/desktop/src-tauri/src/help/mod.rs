//! In-app help assistant (`odd/tasks/in-app-help-assistant.md`): a chat that
//! answers from the bundled user manual and, when the user opts in, from a
//! scrubbed diagnostic snapshot, on the user's own OpenRouter key.

pub(crate) mod diagnostics;
pub(crate) mod manual;
