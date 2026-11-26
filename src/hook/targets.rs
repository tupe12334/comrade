//! Finds the files that a normalized polyhook event writes to.
//!
//! polyhook maps file tools to `write_file` and `edit_file`, and shell calls
//! to `bash`. Two Claude Code tools reach comrade under their own names,
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

/// A path that an event writes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenFile {
    /// The path, as the agent sent it. Relative paths are not resolved here.
    pub path: String,
    /// True when the path is known exactly. False means the rules must apply.
    ///
    /// File tools give exact paths. A Bash target is exact only when it is a
    /// literal path and the command does not change directory or target
    /// another folder.
    pub exact: bool,
}

/// Returns the paths that the event writes to.
///
/// Returns an empty list when the event writes no file.
pub fn written_files(event: &HookEvent) -> Result<Vec<WrittenFile>> {
    let Some(tool) = event.tool.as_deref() else {
        return Ok(Vec::new());
    };
    let input = event.input.as_ref();

    if EDIT_TOOLS.contains(&tool) || UNMAPPED_EDIT_TOOLS.contains(&tool) {
        let path = PATH_KEYS
            .iter()
            .find_map(|key| string_field(input, key))
            .ok_or_else(|| Error::HookInput(format!("`{tool}` input has no string path field")))?;
        return Ok(vec![WrittenFile { path, exact: true }]);
    }

    if tool == "bash" {
        let command = string_field(input, "command")
            .ok_or_else(|| Error::HookInput("`bash` input has no string `command`".to_string()))?;
        let command_exact = !command_moves_target(&command);
        return Ok(shell_writes::write_targets(&command)
            .into_iter()
            .map(|path| WrittenFile {
                exact: command_exact && is_literal_path(&path),
                path,
            })
            .collect());
    }

    Ok(Vec::new())
}

/// True when a shell path holds syntax that the hook cannot expand.
///
/// `~`, `$`, backticks, globs and braces need a shell to resolve them.
fn is_literal_path(path: &str) -> bool {
    !path.contains(['~', '$', '`', '*', '?', '[', '{'])
}

/// True when the command can change where a relative target lands.
///
/// Looks for `cd`, `pushd` or `popd`, and for any option that sets a target
/// folder (`-t`, or a flag group that holds `t`, or `--target-directory`). It
/// over-matches on purpose, so an unclear command fails closed.
fn command_moves_target(command: &str) -> bool {
    command
        .split(|c: char| c.is_whitespace() || ";&|()`".contains(c))
        .any(|word| {
            matches!(word, "cd" | "pushd" | "popd")
                || word == "--target-directory"
                || word.starts_with("--target-directory=")
                || (word.starts_with('-') && !word.starts_with("--") && word.contains('t'))
        })
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
        Ok(written_files(&parsed.event)?
            .into_iter()
            .map(|f| f.path)
            .collect())
    }

    fn exact_flags(json: &str) -> Vec<bool> {
        let parsed = input::read(json.as_bytes()).expect("polyhook reads the payload");
        written_files(&parsed.event)
            .unwrap()
            .into_iter()
            .map(|f| f.exact)
            .collect()
    }

    #[test]
    fn file_tool_targets_are_exact() {
        assert_eq!(
            exact_flags(r#"{"tool_name":"Write","tool_input":{"file_path":"/p/a.rs"}}"#),
            vec![true]
        );
    }

    #[test]
    fn literal_absolute_bash_target_is_exact() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"echo x > /tmp/s/step.sh"}}"#;
        assert_eq!(exact_flags(json), vec![true]);
    }

    #[test]
    fn tilde_target_is_not_exact() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"echo x > ~/repo/src/zz.rs"}}"#;
        assert_eq!(exact_flags(json), vec![false]);
    }

    #[test]
    fn variable_target_is_not_exact() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"echo x > $HOME/repo/zz.rs"}}"#;
        assert_eq!(exact_flags(json), vec![false]);
    }

    #[test]
    fn cd_makes_bash_targets_not_exact() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"cd /repo && tee src/main.rs"}}"#;
        assert_eq!(exact_flags(json), vec![false]);
    }

    #[test]
    fn target_directory_option_makes_bash_targets_not_exact() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"cp -t /repo/src a.rs"}}"#;
        assert_eq!(exact_flags(json), vec![false]);
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
