//! The hook pipeline: read input, load config, decide, and build the response.
//!
//! The pipeline never panics and never returns an error to the caller. Any
//! failure becomes an outcome that follows `on_error` from the config.

use std::path::PathBuf;

use crate::agents::AgentIntegration;
use crate::config::{self, Action, LoadOptions};
use crate::error::Result;
use crate::paths;
use crate::rules::{Engine, Verdict};

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

/// Runs the hook for one agent and one stdin payload.
pub fn run(agent: &dyn AgentIntegration, stdin: &str, options: &HookOptions) -> HookOutcome {
    match decide(agent, stdin, options) {
        Ok(outcome) => outcome,
        Err(error) => on_error_outcome(&error.to_string(), options),
    }
}

fn decide(agent: &dyn AgentIntegration, stdin: &str, options: &HookOptions) -> Result<HookOutcome> {
    let Some(request) = agent.parse(stdin)? else {
        return Ok(HookOutcome::allow());
    };

    let cwd = request.cwd.clone().or_else(|| std::env::current_dir().ok());
    let root = cwd.as_deref().map(paths::project_root);

    let config = config::load(&LoadOptions {
        config_dir: paths::config_dir(options.config_dir.as_deref())?,
        project_root: root.clone(),
    })?;
    if !config.enabled {
        return Ok(HookOutcome::allow());
    }
    let engine = Engine::new(&config)?;

    let model = options.model_override.clone().or_else(|| {
        request
            .session
            .as_ref()
            .and_then(|session| agent.detect_model(session))
    });

    for file in &request.files {
        let evaluation = engine.evaluate(file, root.as_deref(), model.as_deref());
        if let Verdict::Block { message } = evaluation.verdict {
            return Ok(HookOutcome {
                exit_code: 0,
                stdout: agent.block(&message).stdout,
                stderr: String::new(),
            });
        }
    }
    Ok(HookOutcome::allow())
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
    use crate::agents::claude_code::ClaudeCode;

    #[test]
    fn unrelated_tool_is_allowed_without_reading_config() {
        let options = HookOptions {
            config_dir: Some(PathBuf::from("/nonexistent/modelgate-test")),
            model_override: None,
        };
        let outcome = run(
            &ClaudeCode,
            r#"{"tool_name":"Read","tool_input":{"file_path":"/p/a.rs"}}"#,
            &options,
        );
        assert_eq!(outcome, HookOutcome::allow());
    }

    #[test]
    fn bad_json_follows_on_error_default_allow() {
        let tmp = tempfile::tempdir().unwrap();
        let options = HookOptions {
            config_dir: Some(tmp.path().to_path_buf()),
            model_override: None,
        };
        let outcome = run(&ClaudeCode, "{nope", &options);
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.stderr.contains("Allowing the edit"));
    }

    #[test]
    fn bad_json_follows_on_error_block_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "on_error = \"block\"\n").unwrap();
        let options = HookOptions {
            config_dir: Some(tmp.path().to_path_buf()),
            model_override: None,
        };
        let outcome = run(&ClaudeCode, "{nope", &options);
        assert_eq!(outcome.exit_code, 2);
        assert!(outcome.stderr.contains("Blocking the edit"));
    }

    #[test]
    fn broken_user_config_falls_back_to_allow() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "on_error = [").unwrap();
        let options = HookOptions {
            config_dir: Some(tmp.path().to_path_buf()),
            model_override: None,
        };
        let outcome = run(&ClaudeCode, "{nope", &options);
        assert_eq!(outcome.exit_code, 0);
    }
}
