//! Error type shared by the whole crate.

use std::path::PathBuf;

/// Every error that modelgate can report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file could not be read or written.
    #[error("cannot access {path}: {source}")]
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The underlying OS error.
        source: std::io::Error,
    },

    /// A TOML file could not be parsed.
    #[error("invalid TOML in {source_name}: {source}")]
    Toml {
        /// Where the TOML came from (a path or a label).
        source_name: String,
        /// The parser error, with line and column.
        source: toml::de::Error,
    },

    /// A config file parsed but its content is not valid.
    #[error("invalid config in {source_name}: {message}")]
    Config {
        /// Where the bad value came from.
        source_name: String,
        /// What is wrong.
        message: String,
    },

    /// The hook input from the agent is not usable.
    #[error("invalid hook input: {0}")]
    HookInput(String),

    /// The agent name given on the command line is not known.
    #[error("unknown agent `{0}`; known agents: claude")]
    UnknownAgent(String),

    /// The config directory cannot be found (no home directory).
    #[error("cannot find a config directory: {0}")]
    NoConfigDir(String),

    /// Settings file for an agent is not valid JSON or has an unexpected shape.
    #[error("cannot update {path}: {message}")]
    Settings {
        /// The settings file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
}

/// Shorthand result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;
