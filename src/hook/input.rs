//! Reads the hook input through polyhook.
//!
//! polyhook detects the agent and normalizes the event. Its event does not
//! carry the working directory or the transcript path. Those two top-level
//! fields are read here, from the same payload.

use std::path::PathBuf;

use polyhook::HookEvent;
use serde_json::Value;

use crate::error::{Error, Result};

/// The hook input, as comrade needs it.
#[derive(Debug, Clone)]
pub struct HookInput {
    /// The normalized event from polyhook.
    pub event: HookEvent,
    /// Working directory of the session, from the top-level `cwd` field.
    pub cwd: Option<PathBuf>,
    /// Session transcript, from the top-level `transcript_path` field.
    pub transcript_path: Option<PathBuf>,
}

/// Parses the raw stdin bytes.
///
/// polyhook keeps the detected agent in thread-local state for the response.
/// Call [`super::response::block`] on the same thread.
pub fn read(stdin: &[u8]) -> Result<HookInput> {
    let mut reader = stdin;
    let event = polyhook::read_from(&mut reader).map_err(Error::HookInput)?;
    let raw: Value = serde_json::from_slice(stdin)
        .map_err(|e| Error::HookInput(format!("not valid JSON: {e}")))?;
    Ok(HookInput {
        event,
        cwd: top_level_path(&raw, "cwd"),
        transcript_path: top_level_path(&raw, "transcript_path"),
    })
}

/// Parses the payload with the agent environment variables unset.
///
/// polyhook checks environment variables such as `CLAUDE_CODE_VERSION`
/// before the payload shape. Tests that run inside an agent session would
/// otherwise detect that agent instead of the one in the payload.
#[cfg(test)]
pub(crate) fn read_without_agent_env(stdin: &[u8]) -> Result<HookInput> {
    temp_env::with_vars_unset(AGENT_ENV_VARS, || read(stdin))
}

/// Environment variables that polyhook reads to detect the agent.
#[cfg(test)]
const AGENT_ENV_VARS: [&str; 7] = [
    "POLYHOOK_CALLER",
    "CLAUDE_CODE_VERSION",
    "CURSOR_SESSION_ID",
    "WINDSURF_SESSION_ID",
    "CLINE_SESSION_ID",
    "AMP_SESSION_ID",
    "GEMINI_PROJECT_DIR",
];

fn top_level_path(raw: &Value, key: &str) -> Option<PathBuf> {
    raw.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use polyhook::CallerKind;

    #[test]
    fn claude_payload_gives_event_cwd_and_transcript() {
        let input = read_without_agent_env(
            br#"{"hook_event_name":"PreToolUse","session_id":"s1","agent_id":"a1","transcript_path":"/t/s.jsonl","cwd":"/proj","tool_name":"Edit","tool_input":{"file_path":"/proj/a.rs"}}"#,
        )
        .unwrap();
        assert_eq!(input.event.caller, CallerKind::ClaudeCode);
        assert_eq!(input.event.tool.as_deref(), Some("edit_file"));
        assert_eq!(input.event.agent_id.as_deref(), Some("a1"));
        assert_eq!(input.cwd, Some(PathBuf::from("/proj")));
        assert_eq!(input.transcript_path, Some(PathBuf::from("/t/s.jsonl")));
    }

    #[test]
    fn missing_top_level_paths_are_none() {
        let input =
            read_without_agent_env(br#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#)
                .unwrap();
        assert!(input.cwd.is_none());
        assert!(input.transcript_path.is_none());
    }

    #[test]
    fn blank_paths_are_none() {
        let input =
            read_without_agent_env(br#"{"cwd":"  ","transcript_path":"","tool_name":"Bash"}"#)
                .unwrap();
        assert!(input.cwd.is_none());
        assert!(input.transcript_path.is_none());
    }

    #[test]
    fn invalid_json_is_an_error() {
        let err = read_without_agent_env(b"{nope").unwrap_err();
        assert!(err.to_string().contains("JSON parse error"), "{err}");
    }
}
