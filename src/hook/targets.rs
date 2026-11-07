//! Finds the files that a normalized polyhook event writes to.
//!
//! polyhook maps file tools to `write_file` and `edit_file`, and shell calls
//! to `bash`. Two Claude Code tools reach modelgate under their own names,
//! because polyhook does not map them yet: `MultiEdit` and `NotebookEdit`.
//!
//! polyhook does not normalize the path key of a file tool, so the path is
//! read from the first of [`PATH_KEYS`] that holds a string.

use polyhook::HookEvent;
use serde_json::{Map, Value};

use super::shell_writes;
use crate::error::{Error, Result};

/// Normalized file-write tools.
const EDIT_TOOLS: &[&str] = &["write_file", "edit_file"];

/// Edit tools that polyhook passes through with their vendor name.
const UNMAPPED_EDIT_TOOLS: &[&str] = &["MultiEdit", "NotebookEdit"];

/// Input keys that can hold the path of the file a tool writes.
const PATH_KEYS: &[&str] = &["file_path", "path", "notebook_path", "target_file"];

/// Returns the paths that the event writes to, as the agent sent them.
///
/// Returns an empty list when the event writes no file. Relative paths are
/// not resolved here.
pub fn written_files(event: &HookEvent) -> Result<Vec<String>> {
    let Some(tool) = event.tool.as_deref() else {
        return Ok(Vec::new());
    };
    let input = event.input.as_ref();

    if EDIT_TOOLS.contains(&tool) || UNMAPPED_EDIT_TOOLS.contains(&tool) {
        let path = PATH_KEYS
            .iter()
            .find_map(|key| string_field(input, key))
            .ok_or_else(|| Error::HookInput(format!("`{tool}` input has no string path field")))?;
        return Ok(vec![path]);
    }

    if tool == "bash" {
        let command = string_field(input, "command")
            .ok_or_else(|| Error::HookInput("`bash` input has no string `command`".to_string()))?;
        return Ok(shell_writes::write_targets(&command));
    }

    Ok(Vec::new())
}

fn string_field(input: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    input
        .and_then(|map| map.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::input;

    fn files(json: &str) -> Result<Vec<String>> {
        let parsed = input::read(json.as_bytes()).expect("polyhook reads the payload");
        written_files(&parsed.event)
    }

    #[test]
    fn claude_edit_and_write_read_file_path() {
        for tool in ["Edit", "Write", "MultiEdit"] {
            let json = format!(
                r#"{{"hook_event_name":"PreToolUse","tool_name":"{tool}","tool_input":{{"file_path":"/p/x.ts"}}}}"#
            );
            assert_eq!(files(&json).unwrap(), vec!["/p/x.ts"], "{tool}");
        }
    }

    #[test]
    fn notebook_edit_reads_notebook_path() {
        let got = files(
            r#"{"tool_name":"NotebookEdit","tool_input":{"notebook_path":"/p/n.ipynb","new_source":"x"}}"#,
        )
        .unwrap();
        assert_eq!(got, vec!["/p/n.ipynb"]);
    }

    #[test]
    fn cursor_edit_file_reads_path_key() {
        let got = files(
            r#"{"type":"BeforeToolCall","toolCall":{"name":"edit_file","args":{"path":"/p/a.py"}},"sessionId":"s"}"#,
        )
        .unwrap();
        assert_eq!(got, vec!["/p/a.py"]);
    }

    #[test]
    fn gemini_write_file_reads_file_path() {
        let got = files(
            r#"{"hook_event_name":"BeforeTool","tool_name":"write_file","tool_input":{"file_path":"/p/b.go"},"session_id":"s"}"#,
        )
        .unwrap();
        assert_eq!(got, vec!["/p/b.go"]);
    }

    #[test]
    fn file_tool_without_path_is_an_error() {
        let err = files(r#"{"tool_name":"Edit","tool_input":{}}"#).unwrap_err();
        assert!(err.to_string().contains("path field"), "{err}");
    }

    #[test]
    fn bash_redirect_is_a_write() {
        let got =
            files(r#"{"tool_name":"Bash","tool_input":{"command":"echo x > lib.rs"}}"#).unwrap();
        assert_eq!(got, vec!["lib.rs"]);
    }

    #[test]
    fn bash_without_writes_is_empty() {
        let got = files(r#"{"tool_name":"Bash","tool_input":{"command":"ls -la"}}"#).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn bash_without_command_is_an_error() {
        assert!(files(r#"{"tool_name":"Bash","tool_input":{}}"#).is_err());
    }

    #[test]
    fn other_tools_are_empty() {
        let got = files(r#"{"tool_name":"Read","tool_input":{"file_path":"/p/a.rs"}}"#).unwrap();
        assert!(got.is_empty());
    }
}
