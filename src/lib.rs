//! Blocks code edits from AI agents unless the editing model is allowed for
//! that file type.
//!
//! Config lives in `~/.config/modelgate/config.toml`. A rule lists file globs
//! and the model patterns that may edit them. An agent hook calls
//! [`hook::run`] before each edit. The hook denies the edit when the model is
//! not allowed.
//!
//! Start with [`config::load`] and [`rules::Engine`], or with [`hook::run`]
//! for the full pipeline.

pub mod agents;
pub mod cli;
pub mod commands;
pub mod config;
pub mod error;
pub mod hook;
pub mod paths;
pub mod rules;
pub mod template;

pub use error::{Error, Result};
