//! In-app help assistant (`odd/tasks/in-app-help-assistant.md`): a chat that
//! answers from the bundled user manual and, when the user opts in, from a
//! scrubbed diagnostic snapshot, on the user's own OpenRouter key.

pub(crate) mod commands;
pub(crate) mod diagnostics;
pub(crate) mod documents;
pub(crate) mod faq;
pub(crate) mod index;
pub(crate) mod intent;
pub(crate) mod manual;
pub(crate) mod prompt;
pub(crate) mod screenmap;
pub(crate) mod settings;
pub(crate) mod text;
