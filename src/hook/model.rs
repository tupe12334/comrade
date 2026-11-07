//! Finds the model that made the tool call.
//!
//! polyhook does not carry the model name, so the lookup order is:
//!
//! 1. `COMRADE_MODEL`, for every agent.
//! 2. The Claude Code session transcript, for Claude Code calls only. The
//!    `transcript_path` comes from the raw payload. The subagent is taken from
//!    polyhook's `agent_id`.
//!
//! Any other agent without the override has an unknown model.

use polyhook::CallerKind;

use super::input::HookInput;
use crate::agents::claude_code::transcript;

/// Returns the model that made the call, or `None` when it is unknown.
///
/// `model_override` is the non-empty `COMRADE_MODEL` value, if set.
pub fn detect(input: &HookInput, model_override: Option<&str>) -> Option<String> {
    if let Some(model) = model_override {
        return Some(model.to_string());
    }
    if input.event.caller != CallerKind::ClaudeCode {
        return None;
    }
    let transcript_path = input.transcript_path.as_deref()?;
    transcript::detect(transcript_path, input.event.agent_id.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::input;

    fn read(json: &str) -> HookInput {
        input::read(json.as_bytes()).unwrap()
    }

    #[test]
    fn override_wins_for_any_agent() {
        let cursor = read(
            r#"{"type":"BeforeToolCall","toolCall":{"name":"edit_file","args":{}},"sessionId":"s"}"#,
        );
        assert_eq!(
            detect(&cursor, Some("claude-haiku-5-5")).as_deref(),
            Some("claude-haiku-5-5")
        );
    }

    #[test]
    fn other_agents_without_override_are_unknown() {
        let cursor = read(
            r#"{"type":"BeforeToolCall","toolCall":{"name":"edit_file","args":{}},"sessionId":"s","transcript_path":"/nonexistent/t.jsonl"}"#,
        );
        assert_eq!(detect(&cursor, None), None);
    }

    #[test]
    fn claude_reads_transcript_when_no_override() {
        let tmp = tempfile::tempdir().unwrap();
        let session = tmp.path().join("sess.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5"}}"#,
        )
        .unwrap();
        let payload = format!(
            r#"{{"hook_event_name":"PreToolUse","tool_name":"Edit","transcript_path":"{}","tool_input":{{"file_path":"/p/a.rs"}}}}"#,
            session.display()
        );
        assert_eq!(
            detect(&read(&payload), None).as_deref(),
            Some("claude-opus-5-5")
        );
    }

    #[test]
    fn claude_subagent_uses_agent_id_from_polyhook() {
        let tmp = tempfile::tempdir().unwrap();
        let session = tmp.path().join("sess.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5"}}"#,
        )
        .unwrap();
        let sub_dir = tmp.path().join("sess").join("subagents");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(
            sub_dir.join("agent-a1.jsonl"),
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5"}}"#,
        )
        .unwrap();
        let payload = format!(
            r#"{{"hook_event_name":"PreToolUse","tool_name":"Edit","agent_id":"a1","transcript_path":"{}","tool_input":{{"file_path":"/p/a.rs"}}}}"#,
            session.display()
        );
        assert_eq!(
            detect(&read(&payload), None).as_deref(),
            Some("claude-haiku-5-5")
        );
    }

    #[test]
    fn claude_without_transcript_is_unknown() {
        let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"/p/a.rs"}}"#;
        assert_eq!(detect(&read(payload), None), None);
    }
}
