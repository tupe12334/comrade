//! Blocks code edits from AI agents unless the editing model is allowed for
//! that file type.

pub mod error;
pub mod paths;
pub mod template;

pub use error::{Error, Result};
