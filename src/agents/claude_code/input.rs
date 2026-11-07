//! Parses the PreToolUse JSON that Claude Code sends on stdin.
//!
//! Field names come from the Claude Code hooks reference
//! (<https://code.claude.com/docs/en/hooks>). Documented: `hook_event_name`,
//! `transcript_path`, `cwd`, `tool_name`, `tool_input`, `agent_id`,
//! `agent_type`, and `tool_input.file_path` for Edit and Write.
//! `MultiEdit` (`file_path`) and `NotebookEdit` (`notebook_path`) follow the
//! Claude Code tool schemas. Those were not confirmed in the docs page read.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::agents::{EditRequest, SessionRef};
use crate::error::{Error, Result};
use crate::hook::shell_writes;

/// Tools that write a file named by `tool_input.file_path`.
const FILE_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit"];

/// The subset of the hook input that modelgate reads.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    /// `PreToolUse` for this hook. Other events are ignored.
    pub hook_event_name: Option<String>,
    /// Path of the session transcript (JSONL).
    pub transcript_path: Option<PathBuf>,
    /// Working directory of the session.
    pub cwd: Option<PathBuf>,
    /// Name of the tool, such as `Edit` or `Bash`.
    pub tool_name: Option<String>,
    /// The tool arguments. Its shape depends on the tool.
    pub tool_input: Option<Value>,
    /// Set when the call comes from a subagent.
    pub agent_id: Option<String>,
    /// Subagent type, when set.
    pub agent_type: Option<String>,
}

/// Parses hook input. Returns `Ok(None)` when the call does not write files.
pub fn parse(stdin: &str) -> Result<Option<EditRequest>> {
    let input: HookInput = serde_json::from_str(stdin)
        .map_err(|e| Error::HookInput(format!("not valid JSON: {e}")))?;

    if let Some(event) = &input.hook_event_name {
        if event != "PreToolUse" {
            return Ok(None);
        }
    }

    let tool = input
        .tool_name
        .as_deref()
        .ok_or_else(|| Error::HookInput("missing `tool_name`".to_string()))?;
    let tool_input = input.tool_input.as_ref();

    let raw_files: Vec<String> = if FILE_TOOLS.contains(&tool) {
        vec![string_field(tool_input, "file_path")?]
    } else if tool == "NotebookEdit" {
        vec![string_field(tool_input, "notebook_path")?]
    } else if tool == "Bash" {
        let command = string_field(tool_input, "command")?;
        shell_writes::write_targets(&command)
    } else {
        Vec::new()
    };

    if raw_files.is_empty() {
        return Ok(None);
    }

    let cwd = input.cwd.clone();
    let files = raw_files
        .iter()
        .map(|f| absolute(f, cwd.as_deref()))
        .collect();

    let session = input.transcript_path.map(|transcript_path| SessionRef {
        transcript_path,
        agent_id: input.agent_id.clone().filter(|id| !id.trim().is_empty()),
    });

    Ok(Some(EditRequest {
        files,
        cwd,
        session,
    }))
}

fn string_field(tool_input: Option<&Value>, key: &str) -> Result<String> {
    tool_input
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::HookInput(format!("tool_input has no string `{key}`")))
}

/// Makes a path absolute using the session cwd. Leaves absolute paths alone.
fn absolute(file: &str, cwd: Option<&Path>) -> PathBuf {
    let path = Path::new(file);
    match cwd {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(json: &str) -> Option<EditRequest> {
        parse(json).expect("parse should succeed")
    }

    #[test]
    fn edit_reads_file_path() {
        let req = parse_ok(
            r#"{"hook_event_name":"PreToolUse","tool_name":"Edit","cwd":"/proj",
                "transcript_path":"/t/s.jsonl","tool_input":{"file_path":"/proj/src/a.rs","old_string":"a","new_string":"b"}}"#,
        )
        .unwrap();
        assert_eq!(req.files, vec![PathBuf::from("/proj/src/a.rs")]);
        assert_eq!(
            req.session.unwrap().transcript_path,
            PathBuf::from("/t/s.jsonl")
        );
    }

    #[test]
    fn write_and_multiedit_read_file_path() {
        for tool in ["Write", "MultiEdit"] {
            let json = format!(
                r#"{{"hook_event_name":"PreToolUse","tool_name":"{tool}","tool_input":{{"file_path":"/p/x.ts"}}}}"#
            );
            assert_eq!(
                parse_ok(&json).unwrap().files,
                vec![PathBuf::from("/p/x.ts")],
                "{tool}"
            );
        }
    }

    #[test]
    fn notebook_edit_reads_notebook_path() {
        let req = parse_ok(
            r#"{"tool_name":"NotebookEdit","tool_input":{"notebook_path":"/p/n.ipynb","new_source":"x"}}"#,
        )
        .unwrap();
        assert_eq!(req.files, vec![PathBuf::from("/p/n.ipynb")]);
    }

    #[test]
    fn relative_path_uses_cwd() {
        let req = parse_ok(
            r#"{"tool_name":"Write","cwd":"/proj","tool_input":{"file_path":"src/a.py"}}"#,
        )
        .unwrap();
        assert_eq!(req.files, vec![PathBuf::from("/proj/src/a.py")]);
    }

    #[test]
    fn bash_redirect_becomes_file() {
        let req = parse_ok(
            r#"{"tool_name":"Bash","cwd":"/proj","tool_input":{"command":"echo x > lib.rs"}}"#,
        )
        .unwrap();
        assert_eq!(req.files, vec![PathBuf::from("/proj/lib.rs")]);
    }

    #[test]
    fn bash_without_writes_is_none() {
        assert!(parse_ok(r#"{"tool_name":"Bash","tool_input":{"command":"ls -la"}}"#).is_none());
    }

    #[test]
    fn other_tools_are_none() {
        assert!(parse_ok(r#"{"tool_name":"Read","tool_input":{"file_path":"/p/a.rs"}}"#).is_none());
    }

    #[test]
    fn other_events_are_none() {
        assert!(parse_ok(r#"{"hook_event_name":"Stop","tool_name":"Edit"}"#).is_none());
    }

    #[test]
    fn empty_agent_id_is_ignored() {
        let req = parse_ok(
            r#"{"tool_name":"Edit","transcript_path":"/t/s.jsonl","agent_id":"  ","tool_input":{"file_path":"/p/a.rs"}}"#,
        )
        .unwrap();
        assert!(req.session.unwrap().agent_id.is_none());
    }

    #[test]
    fn agent_id_is_kept() {
        let req = parse_ok(
            r#"{"tool_name":"Edit","transcript_path":"/t/s.jsonl","agent_id":"a9","agent_type":"x","tool_input":{"file_path":"/p/a.rs"}}"#,
        )
        .unwrap();
        assert_eq!(req.session.unwrap().agent_id.as_deref(), Some("a9"));
    }

    #[test]
    fn invalid_json_is_an_error() {
        let err = parse("{not json").unwrap_err();
        assert!(err.to_string().contains("not valid JSON"));
    }

    #[test]
    fn missing_file_path_is_an_error() {
        let err = parse(r#"{"tool_name":"Edit","tool_input":{}}"#).unwrap_err();
        assert!(err.to_string().contains("file_path"));
    }

    #[test]
    fn missing_tool_name_is_an_error() {
        assert!(parse(r#"{"hook_event_name":"PreToolUse"}"#).is_err());
    }
}
