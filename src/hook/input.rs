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
        let input = read(
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
        let input = read(br#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#).unwrap();
        assert!(input.cwd.is_none());
        assert!(input.transcript_path.is_none());
    }

    #[test]
    fn blank_paths_are_none() {
        let input = read(br#"{"cwd":"  ","transcript_path":"","tool_name":"Bash"}"#).unwrap();
        assert!(input.cwd.is_none());
        assert!(input.transcript_path.is_none());
    }

    #[test]
    fn invalid_json_is_an_error() {
        let err = read(b"{nope").unwrap_err();
        assert!(err.to_string().contains("JSON parse error"), "{err}");
    }
}
