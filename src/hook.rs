//! The hook pipeline: read input, load config, decide, and build the response.
//!
//! polyhook reads the agent input and writes the response. This module
//! decides. The pipeline never panics and never returns an error to the
//! caller. Any failure becomes an outcome that follows `on_error` from the
//! config.
//!
//! - [`input`]: reads the payload through polyhook.
//! - [`targets`]: finds the files that a tool event writes.
//! - [`model`]: finds the model that made the call.
//! - [`response`]: builds the block response through polyhook.
//! - [`shell_writes`]: finds the files that a shell command writes.

use std::path::{Path, PathBuf};

use polyhook::HookEventEvent;

use crate::agents;
use crate::config::{self, Action, LoadOptions};
use crate::error::{Error, Result};
use crate::paths;
use crate::rules::{Engine, Verdict};

pub mod input;
pub mod model;
pub mod response;
pub mod shell_writes;
pub mod targets;

/// Environment variable that polyhook reads to pick the agent.
const CALLER_ENV: &str = "POLYHOOK_CALLER";

/// Options that come from the command line and the environment.
#[derive(Debug, Clone, Default)]
pub struct HookOptions {
    /// Config folder. `None` uses the default location.
    pub config_dir: Option<PathBuf>,
    /// Model to use instead of detecting one. From `MODELGATE_MODEL`.
    pub model_override: Option<String>,
}

/// What the process must do after the hook runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    /// Process exit code.
    pub exit_code: i32,
    /// Text for stdout.
    pub stdout: String,
    /// Text for stderr.
    pub stderr: String,
}

impl HookOutcome {
    fn allow() -> Self {
        Self {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

/// Sets the agent that polyhook uses, from the `hook <agent>` argument.
///
/// polyhook detects the agent from the payload by default. An explicit name
/// is an override. It is ignored when `POLYHOOK_CALLER` is already set.
/// `claude` is an alias for `claude-code`.
pub fn use_agent_hint(name: &str) -> Result<()> {
    let slug = if name == "claude" {
        "claude-code"
    } else {
        name
    };
    if !agents::POLYHOOK_AGENTS
        .iter()
        .any(|agent| agent.name == slug)
    {
        return Err(Error::UnknownAgent(name.to_string()));
    }
    if std::env::var_os(CALLER_ENV).is_none() {
        std::env::set_var(CALLER_ENV, slug);
    }
    Ok(())
}

/// Runs the hook for one stdin payload. Call on the thread that set the agent hint.
pub fn run(stdin: &[u8], options: &HookOptions) -> HookOutcome {
    match decide(stdin, options) {
        Ok(outcome) => outcome,
        Err(error) => on_error_outcome(&error.to_string(), options),
    }
}

fn decide(stdin: &[u8], options: &HookOptions) -> Result<HookOutcome> {
    let input = input::read(stdin)?;
    if input.event.event != HookEventEvent::ToolBefore {
        return Ok(HookOutcome::allow());
    }

    let files = targets::written_files(&input.event)?;
    if files.is_empty() {
        return Ok(HookOutcome::allow());
    }

    let cwd = input.cwd.clone().or_else(|| std::env::current_dir().ok());
    let root = cwd.as_deref().map(paths::project_root);

    let config = config::load(&LoadOptions {
        config_dir: paths::config_dir(options.config_dir.as_deref())?,
        project_root: root.clone(),
    })?;
    if !config.enabled {
        return Ok(HookOutcome::allow());
    }
    let engine = Engine::new(&config)?;
    let model = model::detect(&input, options.model_override.as_deref());

    for raw in &files {
        let file = absolute(raw, cwd.as_deref());
        let evaluation = engine.evaluate(&file, root.as_deref(), model.as_deref());
        if let Verdict::Block { message } = evaluation.verdict {
            return Ok(HookOutcome {
                exit_code: 0,
                stdout: response::block(&message)?,
                stderr: String::new(),
            });
        }
    }
    Ok(HookOutcome::allow())
}

/// Makes a path absolute using the session cwd. Leaves absolute paths alone.
fn absolute(file: &str, cwd: Option<&Path>) -> PathBuf {
    let path = Path::new(file);
    match cwd {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path.to_path_buf(),
    }
}

/// Builds the outcome for a hook failure. Uses `on_error` from the user config.
///
/// If the user config cannot be read, the action is `allow`, so a broken
/// config does not lock the user out.
fn on_error_outcome(message: &str, options: &HookOptions) -> HookOutcome {
    let action = paths::config_dir(options.config_dir.as_deref())
        .and_then(|config_dir| {
            config::load(&LoadOptions {
                config_dir,
                project_root: None,
            })
        })
        .map(|config| config.on_error)
        .unwrap_or(Action::Allow);

    match action {
        Action::Allow => HookOutcome {
            exit_code: 0,
            stdout: String::new(),
            stderr: format!("modelgate: {message}. Allowing the edit (on_error = \"allow\")."),
        },
        Action::Block => HookOutcome {
            exit_code: 2,
            stdout: String::new(),
            stderr: format!("modelgate: {message}. Blocking the edit (on_error = \"block\")."),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(dir: &Path) -> HookOptions {
        HookOptions {
            config_dir: Some(dir.to_path_buf()),
            model_override: None,
        }
    }

    #[test]
    fn unrelated_tool_is_allowed_without_reading_config() {
        let options = HookOptions {
            config_dir: Some(PathBuf::from("/nonexistent/modelgate-test")),
            model_override: None,
        };
        let outcome = run(
            br#"{"tool_name":"Read","tool_input":{"file_path":"/p/a.rs"}}"#,
            &options,
        );
        assert_eq!(outcome, HookOutcome::allow());
    }

    #[test]
    fn non_tool_event_is_allowed_without_reading_config() {
        let options = HookOptions {
            config_dir: Some(PathBuf::from("/nonexistent/modelgate-test")),
            model_override: None,
        };
        let outcome = run(
            br#"{"hook_event_name":"Stop","tool_name":"Edit","tool_input":{"file_path":"/p/a.rs"}}"#,
            &options,
        );
        assert_eq!(outcome, HookOutcome::allow());
    }

    #[test]
    fn bad_json_follows_on_error_default_allow() {
        let tmp = tempfile::tempdir().unwrap();
        let outcome = run(b"{nope", &options(tmp.path()));
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.stderr.contains("Allowing the edit"));
    }

    #[test]
    fn bad_json_follows_on_error_block_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "on_error = \"block\"\n").unwrap();
        let outcome = run(b"{nope", &options(tmp.path()));
        assert_eq!(outcome.exit_code, 2);
        assert!(outcome.stderr.contains("Blocking the edit"));
    }

    #[test]
    fn broken_user_config_falls_back_to_allow() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "on_error = [").unwrap();
        let outcome = run(b"{nope", &options(tmp.path()));
        assert_eq!(outcome.exit_code, 0);
    }

    #[test]
    fn relative_path_is_joined_to_cwd() {
        assert_eq!(
            absolute("src/a.rs", Some(Path::new("/proj"))),
            PathBuf::from("/proj/src/a.rs")
        );
        assert_eq!(
            absolute("/abs/a.rs", Some(Path::new("/proj"))),
            PathBuf::from("/abs/a.rs")
        );
    }
}
