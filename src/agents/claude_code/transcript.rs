//! Finds the model that made the current tool call, from the session transcript.
//!
//! Claude Code does not send the model in the PreToolUse input. The transcript
//! is a JSONL file. Each assistant line has `message.model`.
//!
//! - Main session: `transcript_path` is the session file. The model is the
//!   last assistant line that is not a sidechain line.
//! - Subagent: the hook input has `agent_id`. The subagent writes its own file
//!   at `<session-dir>/subagents/agent-<agent_id>.jsonl`, next to the session
//!   file. The model is the last assistant line in that file.
//!
//! If the subagent file has no model yet, `agent-<agent_id>.meta.json` gives the model.
//! The main session model is not used, because it would be the wrong model.

use std::path::{Path, PathBuf};

/// Returns the model that made the latest tool call, or `None` if unknown.
pub fn detect(transcript: &Path, agent_id: Option<&str>) -> Option<String> {
    let Some(id) = agent_id else {
        let text = std::fs::read_to_string(transcript).ok()?;
        return last_model(&text, false);
    };
    let from_transcript = std::fs::read_to_string(subagent_transcript(transcript, id))
        .ok()
        .and_then(|text| last_model(&text, true));
    from_transcript.or_else(|| meta_model(&subagent_meta(transcript, id)))
}

/// Returns the subagent transcript path for a session transcript and agent id.
pub fn subagent_transcript(session_transcript: &Path, agent_id: &str) -> PathBuf {
    let session_dir = session_transcript.with_extension("");
    session_dir
        .join("subagents")
        .join(format!("agent-{agent_id}.jsonl"))
}

/// Returns the subagent meta path for a session transcript and agent id.
pub fn subagent_meta(session_transcript: &Path, agent_id: &str) -> PathBuf {
    let session_dir = session_transcript.with_extension("");
    session_dir
        .join("subagents")
        .join(format!("agent-{agent_id}.meta.json"))
}

/// Returns the top-level `model` of a subagent meta file, or `None` if unusable.
///
/// `inherit` means the subagent uses the parent model, so it is not a model name.
fn meta_model(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let model = value.get("model")?.as_str()?.trim();
    if model.is_empty() || model.eq_ignore_ascii_case("inherit") {
        None
    } else {
        Some(model.to_string())
    }
}

/// Returns the model of the last usable assistant line in JSONL `text`.
///
/// For the main session (`subagent == false`), sidechain lines are skipped.
/// Synthetic entries such as `<synthetic>` are skipped in both cases.
pub fn last_model(text: &str, subagent: bool) -> Option<String> {
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| entry.get("type").and_then(|t| t.as_str()) == Some("assistant"))
        .filter(|entry| {
            subagent || entry.get("isSidechain").and_then(|s| s.as_bool()) != Some(true)
        })
        .find_map(|entry| {
            let model = entry.pointer("/message/model")?.as_str()?.trim();
            if model.is_empty() || model.starts_with('<') {
                None
            } else {
                Some(model.to_string())
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = r#"{"type":"user","message":{"role":"user","content":"hi"}}
{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5","content":[]}}
{"type":"user","message":{"role":"user","content":"more"}}
{"type":"assistant","isSidechain":false,"message":{"model":"claude-sonnet-5","content":[]}}
not json at all
{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5","content":[]}}
"#;

    #[test]
    fn main_session_uses_last_non_sidechain_assistant() {
        assert_eq!(last_model(MAIN, false).as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn subagent_file_uses_last_assistant_including_sidechain() {
        assert_eq!(last_model(MAIN, true).as_deref(), Some("claude-haiku-5-5"));
    }

    #[test]
    fn synthetic_entries_are_skipped() {
        let text = r#"{"type":"assistant","message":{"model":"claude-haiku-5-5"}}
{"type":"assistant","message":{"model":"<synthetic>"}}"#;
        assert_eq!(last_model(text, false).as_deref(), Some("claude-haiku-5-5"));
    }

    #[test]
    fn no_assistant_line_means_unknown() {
        let text = r#"{"type":"user","message":{"role":"user","content":"x"}}"#;
        assert_eq!(last_model(text, false), None);
    }

    #[test]
    fn subagent_path_is_next_to_session_file() {
        let session = Path::new("/home/u/.claude/projects/p/abc123.jsonl");
        assert_eq!(
            subagent_transcript(session, "def456"),
            PathBuf::from("/home/u/.claude/projects/p/abc123/subagents/agent-def456.jsonl")
        );
    }

    #[test]
    fn detect_reads_subagent_file_and_not_main_session() {
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
            r#"{"type":"assistant","isSidechain":true,"agentId":"a1","message":{"model":"claude-haiku-5-5"}}"#,
        )
        .unwrap();

        assert_eq!(detect(&session, None).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(
            detect(&session, Some("a1")).as_deref(),
            Some("claude-haiku-5-5")
        );
        assert_eq!(detect(&session, Some("missing")), None);
    }

    #[test]
    fn detect_missing_file_is_unknown() {
        assert_eq!(detect(Path::new("/nonexistent/x.jsonl"), None), None);
    }

    /// Returns a session path whose `subagents` folder exists in a unique temp dir.
    fn temp_session(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("comrade-transcript-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sess").join("subagents")).unwrap();
        dir.join("sess.jsonl")
    }

    #[test]
    fn subagent_without_assistant_line_uses_meta_model() {
        let session = temp_session("meta-fallback");
        let sub_dir = session.with_extension("").join("subagents");
        std::fs::write(
            sub_dir.join("agent-a1.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":"x"}}"#,
        )
        .unwrap();
        std::fs::write(
            sub_dir.join("agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","model":"haiku","pickedEffortModel":"claude-opus-5-5"}"#,
        )
        .unwrap();

        assert_eq!(detect(&session, Some("a1")).as_deref(), Some("haiku"));
        let _ = std::fs::remove_dir_all(session.parent().unwrap());
    }

    #[test]
    fn transcript_model_wins_over_meta_model() {
        let session = temp_session("transcript-wins");
        let sub_dir = session.with_extension("").join("subagents");
        std::fs::write(
            sub_dir.join("agent-a1.jsonl"),
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5"}}"#,
        )
        .unwrap();
        std::fs::write(sub_dir.join("agent-a1.meta.json"), r#"{"model":"sonnet"}"#).unwrap();

        assert_eq!(
            detect(&session, Some("a1")).as_deref(),
            Some("claude-haiku-5-5")
        );
        let _ = std::fs::remove_dir_all(session.parent().unwrap());
    }

    #[test]
    fn inherit_meta_model_is_unknown() {
        let session = temp_session("meta-inherit");
        let sub_dir = session.with_extension("").join("subagents");
        std::fs::write(sub_dir.join("agent-a1.meta.json"), r#"{"model":"inherit"}"#).unwrap();

        assert_eq!(detect(&session, Some("a1")), None);
        let _ = std::fs::remove_dir_all(session.parent().unwrap());
    }
}
