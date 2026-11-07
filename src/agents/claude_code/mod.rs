//! Claude Code integration.
//!
//! Hook: `PreToolUse`, matcher `Edit|Write|MultiEdit|NotebookEdit|Bash`.
//!
//! - Input: JSON on stdin. See [`input`].
//! - Model: read from the transcript named in `transcript_path`. Subagent
//!   calls read the subagent's own transcript. See [`transcript`].
//! - Block: JSON on stdout with `permissionDecision: "deny"`. See [`output`].
//! - Settings: [`install`] writes the hook entry into `settings.json`.

pub mod bash_writes;
pub mod input;
pub mod install;
pub mod output;
pub mod transcript;

use crate::agents::{AgentIntegration, EditRequest, HookResponse, SessionRef};
use crate::error::Result;

/// Claude Code. Unit struct that implements [`AgentIntegration`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCode;

impl AgentIntegration for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn parse(&self, stdin: &str) -> Result<Option<EditRequest>> {
        input::parse(stdin)
    }

    fn detect_model(&self, session: &SessionRef) -> Option<String> {
        transcript::detect(&session.transcript_path, session.agent_id.as_deref())
    }

    fn block(&self, message: &str) -> HookResponse {
        output::deny(message)
    }
}
