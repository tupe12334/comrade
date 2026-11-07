//! Agent integrations.
//!
//! Each agent lives in its own module, with its own input, output, and
//! transcript code. An agent implements [`AgentIntegration`]. The rest of the
//! crate only sees that trait.
//!
//! Adding an agent: create `src/agents/<agent>/mod.rs`, implement the trait,
//! and add it to [`by_name`]. See the README.

pub mod claude_code;

use std::path::PathBuf;

use crate::error::{Error, Result};

/// One file-edit request, as an agent describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    /// Absolute paths that the tool call writes to.
    pub files: Vec<PathBuf>,
    /// Working directory of the session, if the agent sends it.
    pub cwd: Option<PathBuf>,
    /// Where to find the model that made the call.
    pub session: Option<SessionRef>,
}

/// Pointer to the agent transcript that names the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    /// The session transcript file (JSONL).
    pub transcript_path: PathBuf,
    /// Set when the call comes from a subagent. The subagent has its own file.
    pub agent_id: Option<String>,
}

/// What the hook prints to stdout. Exit code is always 0 for a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookResponse {
    /// Text for the agent runtime.
    pub stdout: String,
}

/// The contract that each agent integration implements.
pub trait AgentIntegration: Sync {
    /// Short name used on the command line, such as `claude`.
    fn id(&self) -> &'static str;

    /// Reads the hook input. Returns `Ok(None)` when the call writes no file.
    fn parse(&self, stdin: &str) -> Result<Option<EditRequest>>;

    /// Returns the model that made the call, or `None` when it is unknown.
    fn detect_model(&self, session: &SessionRef) -> Option<String>;

    /// Returns the response that blocks the call and shows `message` to the agent.
    fn block(&self, message: &str) -> HookResponse;
}

/// Known agent names, for help text and errors.
pub const AGENT_NAMES: &[&str] = &["claude"];

/// Looks up an agent by its short name.
pub fn by_name(name: &str) -> Result<&'static dyn AgentIntegration> {
    match name {
        "claude" | "claude-code" => Ok(&claude_code::ClaudeCode),
        other => Err(Error::UnknownAgent(other.to_string())),
    }
}
