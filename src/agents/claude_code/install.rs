//! Adds the PreToolUse hook entry to Claude Code settings.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};

/// The command Claude Code runs. It reads the hook input on stdin.
pub const HOOK_COMMAND: &str = "modelgate hook claude";

/// Tools that the hook checks.
pub const TOOL_MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash";

/// Outcome of writing the settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The hook entry was added. `backup` is the copy of the old file, if any.
    Added {
        /// The settings file that was written.
        path: PathBuf,
        /// The backup of the previous file, when the file existed.
        backup: Option<PathBuf>,
    },
    /// The hook entry was already there. Nothing changed.
    AlreadyInstalled {
        /// The settings file that was checked.
        path: PathBuf,
    },
}

/// Returns the hook entry as JSON. It goes under `hooks.PreToolUse`.
pub fn hook_entry() -> Value {
    json!({
        "matcher": TOOL_MATCHER,
        "hooks": [
            { "type": "command", "command": HOOK_COMMAND }
        ]
    })
}

/// Returns the full settings snippet that `install claude` prints.
pub fn snippet() -> String {
    let body = json!({
        "hooks": {
            "PreToolUse": [hook_entry()]
        }
    });
    serde_json::to_string_pretty(&body).unwrap_or_default()
}

/// Returns the Claude Code settings file.
///
/// Uses `$CLAUDE_CONFIG_DIR/settings.json` when set, else `$HOME/.claude/settings.json`.
pub fn settings_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join("settings.json"));
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| Error::NoConfigDir("HOME is not set".to_string()))?;
    Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

/// Adds the hook entry to the settings file at `path`.
///
/// Keeps every other key and hook. Writes a backup first when the file exists.
/// Does nothing when a modelgate entry is already present.
pub fn write_settings(path: &Path) -> Result<WriteOutcome> {
    let existed = path.is_file();
    let mut settings = read_settings(path, existed)?;

    if has_modelgate_entry(&settings) {
        return Ok(WriteOutcome::AlreadyInstalled {
            path: path.to_path_buf(),
        });
    }

    let hooks = object_entry(&mut settings, "hooks", path)?;
    let pre_tool = array_entry(hooks, "PreToolUse", path)?;
    pre_tool.push(hook_entry());

    let backup = if existed {
        let backup = backup_path(path);
        std::fs::copy(path, &backup).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Some(backup)
    } else {
        None
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut text = serde_json::to_string_pretty(&settings).map_err(|e| Error::Settings {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    text.push('\n');
    let tmp = path.with_extension("json.modelgate-tmp");
    std::fs::write(&tmp, text).map_err(|source| Error::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(WriteOutcome::Added {
        path: path.to_path_buf(),
        backup,
    })
}

fn read_settings(path: &Path, existed: bool) -> Result<Value> {
    if !existed {
        return Ok(Value::Object(Map::new()));
    }
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_str(&text).map_err(|e| Error::Settings {
        path: path.to_path_buf(),
        message: format!("not valid JSON, not changed: {e}"),
    })?;
    if !value.is_object() {
        return Err(Error::Settings {
            path: path.to_path_buf(),
            message: "top level is not a JSON object, not changed".to_string(),
        });
    }
    Ok(value)
}

fn has_modelgate_entry(settings: &Value) -> bool {
    settings
        .pointer("/hooks/PreToolUse")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|c| c.contains(HOOK_COMMAND))
                        })
                    })
            })
        })
}

fn object_entry<'a>(
    parent: &'a mut Value,
    key: &str,
    path: &Path,
) -> Result<&'a mut Map<String, Value>> {
    let map = parent.as_object_mut().ok_or_else(|| Error::Settings {
        path: path.to_path_buf(),
        message: "expected a JSON object".to_string(),
    })?;
    let entry = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    entry.as_object_mut().ok_or_else(|| Error::Settings {
        path: path.to_path_buf(),
        message: format!("`{key}` is not a JSON object, not changed"),
    })
}

fn array_entry<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
    path: &Path,
) -> Result<&'a mut Vec<Value>> {
    let entry = parent
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    entry.as_array_mut().ok_or_else(|| Error::Settings {
        path: path.to_path_buf(),
        message: format!("`hooks.{key}` is not a JSON array, not changed"),
    })
}

fn backup_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".modelgate-backup-{stamp}"));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_contains_matcher_and_command() {
        let text = snippet();
        assert!(text.contains(TOOL_MATCHER));
        assert!(text.contains(HOOK_COMMAND));
        assert!(text.contains("PreToolUse"));
    }

    #[test]
    fn creates_missing_settings_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude").join("settings.json");
        let outcome = write_settings(&path).unwrap();
        assert_eq!(
            outcome,
            WriteOutcome::Added {
                path: path.clone(),
                backup: None
            }
        );
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["hooks"]["PreToolUse"][0]["matcher"], TOOL_MATCHER);
    }

    #[test]
    fn keeps_existing_hooks_and_writes_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"other"}]}],"Stop":[]}}"#,
        )
        .unwrap();

        let outcome = write_settings(&path).unwrap();
        let WriteOutcome::Added { backup, .. } = outcome else {
            panic!("expected Added");
        };
        let backup = backup.expect("backup for existing file");
        assert!(std::fs::read_to_string(&backup)
            .unwrap()
            .contains("\"other\""));

        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["model"], "opus");
        assert!(value["hooks"]["Stop"].is_array());
        let pre = value["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0]["hooks"][0]["command"], "other");
        assert_eq!(pre[1]["hooks"][0]["command"], HOOK_COMMAND);
    }

    #[test]
    fn second_run_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        write_settings(&path).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let outcome = write_settings(&path).unwrap();
        assert!(matches!(outcome, WriteOutcome::AlreadyInstalled { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn invalid_json_is_not_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, "{ broken").unwrap();
        assert!(write_settings(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
    }
}
