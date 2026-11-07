//! Agent-specific code that polyhook does not cover.
//!
//! polyhook reads each agent's hook input and writes its response. What is
//! left per agent is the Claude Code settings installer and the Claude Code
//! model fallback. See [`claude_code`].
//!
//! [`POLYHOOK_AGENTS`] lists the agents that polyhook supports. It is used
//! for the `hook <agent>` argument and for `install` output.

pub mod claude_code;

/// An agent that polyhook supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentInfo {
    /// The polyhook name of the agent, as used by `POLYHOOK_CALLER`.
    pub name: &'static str,
    /// The hook event that runs before a tool call, as the agent names it.
    pub pre_tool_event: &'static str,
    /// The agent's hook documentation.
    pub docs: &'static str,
}

/// Agents that polyhook supports, from polyhook's `tools.toml` and README.
pub const POLYHOOK_AGENTS: &[AgentInfo] = &[
    AgentInfo {
        name: "claude-code",
        pre_tool_event: "PreToolUse",
        docs: "https://code.claude.com/docs/en/hooks",
    },
    AgentInfo {
        name: "cursor",
        pre_tool_event: "BeforeToolCall",
        docs: "https://cursor.com/docs/hooks",
    },
    AgentInfo {
        name: "windsurf",
        pre_tool_event: "pre_tool",
        docs: "https://docs.windsurf.com/windsurf/cascade/hooks",
    },
    AgentInfo {
        name: "cline",
        pre_tool_event: "beforeToolUse",
        docs: "https://github.com/cline/cline",
    },
    AgentInfo {
        name: "amp",
        pre_tool_event: "tool.before",
        docs: "https://ampcode.com/news/hooks",
    },
    AgentInfo {
        name: "gemini-cli",
        pre_tool_event: "BeforeTool",
        docs: "https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/index.md",
    },
    AgentInfo {
        name: "hermes",
        pre_tool_event: "pre_tool_call",
        docs: "https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks",
    },
    AgentInfo {
        name: "pi",
        pre_tool_event: "PreToolUse",
        docs: "https://github.com/eleqtrizit/pi-autohooks",
    },
    AgentInfo {
        name: "codex",
        pre_tool_event: "PreToolUse",
        docs: "https://developers.openai.com/codex/hooks",
    },
];
