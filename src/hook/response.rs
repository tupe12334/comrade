//! Builds the hook response through polyhook.
//!
//! polyhook writes each response in the format of the agent that `read`
//! detected. modelgate only sends blocks. An allow prints nothing.

use polyhook::HookResponse;

use crate::error::{Error, Result};

/// Returns the response text that denies a tool call and shows `message`.
///
/// Call this after [`super::input::read`] on the same thread.
pub fn block(message: &str) -> Result<String> {
    let mut out = Vec::new();
    polyhook::respond_to(&mut out, &HookResponse::block(message)).map_err(Error::HookOutput)?;
    String::from_utf8(out).map_err(|e| Error::HookOutput(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::input;

    #[test]
    fn claude_block_is_a_pre_tool_deny() {
        input::read(
            br#"{"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"file_path":"/p/a.rs"}}"#,
        )
        .unwrap();
        let text = block("Send this edit to a Haiku subagent.").unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let out = &value["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PreToolUse");
        assert_eq!(out["permissionDecision"], "deny");
        assert_eq!(
            out["permissionDecisionReason"],
            "Send this edit to a Haiku subagent."
        );
    }

    #[test]
    fn cursor_block_is_a_deny_action() {
        input::read(
            br#"{"type":"BeforeToolCall","toolCall":{"name":"edit_file","args":{"path":"/p/a.rs"}},"sessionId":"s"}"#,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&block("no").unwrap()).unwrap();
        assert_eq!(value["action"], "deny");
        assert_eq!(value["message"], "no");
    }
}
