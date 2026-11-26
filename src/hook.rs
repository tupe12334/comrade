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
use crate::rules::{outside_repo_skips, Engine, Verdict};

pub mod input;
pub mod model;
pub mod response;
mod shell_fallback;
mod shell_words;
pub mod shell_writes;
pub mod targets;

/// Environment variable that polyhook reads to pick the agent.
const CALLER_ENV: &str = "POLYHOOK_CALLER";

/// Options that come from the command line and the environment.
#[derive(Debug, Clone, Default)]
pub struct HookOptions {
    /// Config folder. `None` uses the default location.
    pub config_dir: Option<PathBuf>,
    /// Model to use instead of detecting one. From `COMRADE_MODEL`.
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
    let agent = agents::find(name).ok_or_else(|| Error::UnknownAgent(name.to_string()))?;
    if std::env::var_os(CALLER_ENV).is_none() {
        std::env::set_var(CALLER_ENV, agent.name);
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

    // Shell targets are never exempt. Their paths are not resolved the way the shell does.
    let may_skip = !targets::is_shell(&input.event);
    for raw in &files {
        let file = absolute(raw, cwd.as_deref());
        if may_skip && outside_repo_skips(config.outside_repo, &file) {
            continue;
        }
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
            stderr: format!("comrade: {message}. Allowing the edit (on_error = \"allow\")."),
        },
        Action::Block => HookOutcome {
            exit_code: 2,
            stdout: String::new(),
            stderr: format!("comrade: {message}. Blocking the edit (on_error = \"block\")."),
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
            config_dir: Some(PathBuf::from("/nonexistent/comrade-test")),
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
            config_dir: Some(PathBuf::from("/nonexistent/comrade-test")),
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

    /// Runs a Write by an Opus model on `file`.
    fn opus_write(file: &Path, config_dir: &Path) -> HookOutcome {
        let options = HookOptions {
            model_override: Some("claude-opus-5-5".to_string()),
            ..options(config_dir)
        };
        let payload = format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{}"}}}}"#,
            file.display()
        );
        run(payload.as_bytes(), &options)
    }

    /// Runs a tool call by an Opus model, with `cwd` as the session folder.
    fn opus_run(tool: &str, input: &str, cwd: &Path, config_dir: &Path) -> HookOutcome {
        let options = HookOptions {
            model_override: Some("claude-opus-5-5".to_string()),
            ..options(config_dir)
        };
        let payload = format!(
            r#"{{"tool_name":"{tool}","tool_input":{input},"cwd":"{}"}}"#,
            cwd.display()
        );
        run(payload.as_bytes(), &options)
    }

    #[test]
    fn opus_write_outside_repo_is_allowed_by_default() {
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let config_dir = tmp.path().join("config");
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let outcome = opus_write(&scratch.join("step.sh"), &config_dir);
        assert_eq!(outcome, HookOutcome::allow());
    }

    #[test]
    fn opus_write_inside_repo_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let outcome = opus_write(&repo.join("step.sh"), &config_dir);
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn opus_write_outside_repo_is_blocked_when_outside_repo_is_block() {
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), "outside_repo = \"block\"\n").unwrap();
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let outcome = opus_write(&scratch.join("step.sh"), &config_dir);
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn bash_tilde_target_outside_repo_is_blocked() {
        // `~` is not expanded, so the path would resolve under the cwd. It must stay blocked.
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let outcome = opus_run(
            "Bash",
            r#"{"command":"echo x > ~/dev/git/github/tupe12334/comrade/src/zz.rs"}"#,
            &scratch,
            &tmp.path().join("config"),
        );
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn bash_variable_target_outside_repo_is_blocked() {
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let outcome = opus_run(
            "Bash",
            r#"{"command":"echo x > $HOME/repo/zz.rs"}"#,
            &scratch,
            &tmp.path().join("config"),
        );
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn bash_cd_then_tee_is_blocked() {
        // Without the cd check, src/main.rs would resolve under the scratch cwd and pass.
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let scratch = tmp.path().join("scratch");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let command = format!("cd {} && tee src/main.rs", repo.display());
        let outcome = opus_run(
            "Bash",
            &format!(r#"{{"command":"{command}"}}"#),
            &scratch,
            &tmp.path().join("config"),
        );
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn bash_write_outside_repo_is_blocked() {
        // Shell targets always keep the rules, even for a literal scratch path.
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let command = format!(r#"{{"command":"cat > {}/step.sh"}}"#, scratch.display());
        let outcome = opus_run("Bash", &command, &scratch, &tmp.path().join("config"));
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn write_tool_outside_repo_is_allowed() {
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let outcome = opus_run(
            "Write",
            &format!(r#"{{"file_path":"{}"}}"#, scratch.join("step.sh").display()),
            &scratch,
            &tmp.path().join("config"),
        );
        assert_eq!(outcome, HookOutcome::allow());
    }

    /// Runs a Write by an Opus model on `path` and returns the outcome.
    fn opus_write_to(path: &Path, cwd: &Path, config_dir: &Path) -> HookOutcome {
        opus_run(
            "Write",
            &format!(r#"{{"file_path":"{}"}}"#, path.display()),
            cwd,
            config_dir,
        )
    }

    #[test]
    fn write_inside_repo_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let outcome = opus_write_to(&repo.join("src/x.rs"), &repo, &tmp.path().join("config"));
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_through_symlink_dotdot_into_repo_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let lnk = tmp.path().join("lnk");
        std::os::unix::fs::symlink(repo.join("src"), &lnk).unwrap();
        let cwd = tmp.path().join("scratch");
        std::fs::create_dir_all(&cwd).unwrap();
        let outcome = opus_write_to(&lnk.join("../a.rs"), &cwd, &tmp.path().join("config"));
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_to_hard_link_of_repo_file_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        std::fs::write(repo.join("src/h.rs"), "x").unwrap();
        let hl = tmp.path().join("hl.rs");
        std::fs::hard_link(repo.join("src/h.rs"), &hl).unwrap();
        let outcome = opus_write_to(&hl, tmp.path(), &tmp.path().join("config"));
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_under_proc_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let outcome = opus_write_to(
            Path::new("/proc/self/cwd/x.rs"),
            tmp.path(),
            &tmp.path().join("config"),
        );
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn write_into_missing_folder_is_blocked() {
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let outcome = opus_write_to(
            &tmp.path().join("new/dir/x.rs"),
            tmp.path(),
            &tmp.path().join("config"),
        );
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
    }

    #[test]
    fn project_config_cannot_turn_the_guard_off() {
        // The user config blocks outside-repo edits. A project file must not allow them.
        let Some(tmp) = paths::git_free_tempdir() else {
            return;
        };
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), "outside_repo = \"block\"\n").unwrap();
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join(".comrade.toml"), "outside_repo = \"allow\"\n").unwrap();
        let outcome = opus_run(
            "Write",
            &format!(r#"{{"file_path":"{}"}}"#, scratch.join("step.sh").display()),
            &scratch,
            &config_dir,
        );
        assert!(outcome.stdout.contains("does not type"), "{outcome:?}");
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
