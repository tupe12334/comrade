//! Builds the stdout that Claude Code reads from a PreToolUse hook.
//!
//! A block uses JSON on stdout with exit code 0:
//! `hookSpecificOutput.permissionDecision` is `"deny"` and
//! `permissionDecisionReason` holds the message for the model.

use serde_json::json;

use crate::agents::HookResponse;

/// Returns the response that denies a tool call and shows `message` to the model.
pub fn deny(message: &str) -> HookResponse {
    let body = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": message,
        }
    });
    HookResponse {
        stdout: body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_has_documented_shape() {
        let response = deny("Send this edit to a Haiku subagent.");
        let value: serde_json::Value = serde_json::from_str(&response.stdout).unwrap();
        let out = &value["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PreToolUse");
        assert_eq!(out["permissionDecision"], "deny");
        assert_eq!(
            out["permissionDecisionReason"],
            "Send this edit to a Haiku subagent."
        );
    }
}
