//! Claude Code specific code.
//!
//! - [`install`]: adds the `PreToolUse` hook entry to `settings.json`.
//! - [`transcript`]: the model fallback. polyhook does not carry the model,
//!   so this reads the session transcript that Claude Code writes.
//!
//! Hook input and output are handled by polyhook. See [`crate::hook`].

pub mod install;
pub mod transcript;
