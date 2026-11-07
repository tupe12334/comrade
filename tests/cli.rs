//! Runs the real binary with sample hook input. Every test uses temp folders
//! for the config, the home directory, and the project. None touches the real
//! `~/.config` or `~/.claude`.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use tempfile::TempDir;

/// Temp folders for one test.
struct Sandbox {
    _root: TempDir,
    config: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let home = root.path().join("home");
        let project = root.path().join("project");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        Self {
            _root: root,
            config,
            home,
            project,
        }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("modelgate").unwrap();
        cmd.env("MODELGATE_CONFIG_DIR", &self.config)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("MODELGATE_MODEL")
            .env_remove("CLAUDE_CONFIG_DIR");
        cmd
    }

    /// Writes a session transcript whose last assistant line uses `model`.
    fn transcript(&self, model: &str) -> PathBuf {
        let path = self.project.join("session.jsonl");
        let line = json!({
            "type": "assistant",
            "isSidechain": false,
            "message": { "model": model, "content": [] }
        });
        std::fs::write(&path, format!("{line}\n")).unwrap();
        path
    }

    /// Writes the subagent transcript for `agent_id` next to the session file.
    fn subagent_transcript(&self, agent_id: &str, model: &str) {
        let dir = self.project.join("session").join("subagents");
        std::fs::create_dir_all(&dir).unwrap();
        let line = json!({
            "type": "assistant",
            "isSidechain": true,
            "agentId": agent_id,
            "message": { "model": model, "content": [] }
        });
        std::fs::write(
            dir.join(format!("agent-{agent_id}.jsonl")),
            format!("{line}\n"),
        )
        .unwrap();
    }

    fn edit_input(&self, transcript: &Path, file: &str, agent_id: Option<&str>) -> String {
        let mut value = json!({
            "hook_event_name": "PreToolUse",
            "session_id": "session",
            "transcript_path": transcript,
            "cwd": self.project,
            "tool_name": "Edit",
            "tool_input": {
                "file_path": self.project.join(file),
                "old_string": "a",
                "new_string": "b"
            }
        });
        if let Some(id) = agent_id {
            value["agent_id"] = json!(id);
            value["agent_type"] = json!("general-purpose");
        }
        value.to_string()
    }
}

fn deny_reason(stdout: &[u8]) -> String {
    let value: serde_json::Value = serde_json::from_slice(stdout).expect("stdout is JSON");
    value["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("reason present")
        .to_string()
}

#[test]
fn haiku_may_edit_rust() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-haiku-5-5");
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", None))
        .assert()
        .success()
        .stdout("")
        .stderr("");
}

#[test]
fn opus_is_blocked_from_rust_with_haiku_message() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-opus-5-5");
    let output = sb
        .cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", None))
        .assert()
        .success()
        .get_output()
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "deny");
    let reason = deny_reason(&output.stdout);
    assert!(
        reason.starts_with("Send this edit to a Haiku subagent."),
        "{reason}"
    );
    assert!(reason.contains("claude-opus-5-5"), "{reason}");
}

#[test]
fn haiku_subagent_is_allowed_even_when_main_model_is_opus() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-opus-5-5");
    sb.subagent_transcript("a1b2", "claude-haiku-5-5");
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", Some("a1b2")))
        .assert()
        .success()
        .stdout("");
}

#[test]
fn subagent_on_opus_is_blocked_even_when_main_model_is_haiku() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-haiku-5-5");
    sb.subagent_transcript("zz", "claude-opus-5-5");
    let output = sb
        .cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", Some("zz")))
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(deny_reason(&output.stdout).contains("claude-opus-5-5"));
}

#[test]
fn missing_subagent_transcript_means_unknown_model_and_block() {
    let sb = Sandbox::new();
    // The main session is Haiku. It must not be used for a subagent with no file.
    let transcript = sb.transcript("claude-haiku-5-5");
    let output = sb
        .cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", Some("nope")))
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(deny_reason(&output.stdout).contains("Could not detect the model"));
}

#[test]
fn model_override_env_is_used() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-opus-5-5");
    sb.cmd()
        .env("MODELGATE_MODEL", "claude-haiku-5-5")
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", None))
        .assert()
        .success()
        .stdout("");
}

#[test]
fn markdown_is_not_blocked_by_default() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-opus-5-5");
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "README.md", None))
        .assert()
        .success()
        .stdout("");
}

#[test]
fn bash_redirect_into_rust_file_is_blocked_for_opus() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-opus-5-5");
    let input = json!({
        "hook_event_name": "PreToolUse",
        "transcript_path": transcript,
        "cwd": sb.project,
        "tool_name": "Bash",
        "tool_input": { "command": "cat > src/main.rs <<'EOF'\nfn main() {}\nEOF" }
    });
    let output = sb
        .cmd()
        .args(["hook", "claude"])
        .write_stdin(input.to_string())
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(deny_reason(&output.stdout).contains("Send this edit to a Haiku subagent."));
}

#[test]
fn bash_without_file_writes_is_allowed() {
    let sb = Sandbox::new();
    let transcript = sb.transcript("claude-opus-5-5");
    let input = json!({
        "tool_name": "Bash",
        "transcript_path": transcript,
        "tool_input": { "command": "cargo test 2>&1 | tail -5" }
    });
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(input.to_string())
        .assert()
        .success()
        .stdout("");
}

#[test]
fn bad_json_allows_by_default_and_says_so() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin("{not json")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("Allowing the edit"));
}

#[test]
fn bad_json_blocks_when_on_error_is_block() {
    let sb = Sandbox::new();
    std::fs::write(sb.config.join("config.toml"), "on_error = \"block\"\n").unwrap();
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin("{not json")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Blocking the edit"));
}

#[test]
fn broken_config_does_not_lock_user_out() {
    let sb = Sandbox::new();
    std::fs::write(sb.config.join("config.toml"), "enabled = [oops\n").unwrap();
    let transcript = sb.transcript("claude-opus-5-5");
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", None))
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("Allowing the edit"));
}

#[test]
fn project_file_overrides_user_config() {
    let sb = Sandbox::new();
    std::fs::write(
        sb.project.join(".modelgate.toml"),
        "[[rule]]\nname = \"source-code\"\nallowed_models = [\"*opus*\"]\n",
    )
    .unwrap();
    let transcript = sb.transcript("claude-opus-5-5");
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", None))
        .assert()
        .success()
        .stdout("");
}

#[test]
fn local_file_overrides_shared_config() {
    let sb = Sandbox::new();
    std::fs::write(sb.config.join("config.toml"), "enabled = true\n").unwrap();
    std::fs::write(sb.config.join("config.local.toml"), "enabled = false\n").unwrap();
    let transcript = sb.transcript("claude-opus-5-5");
    sb.cmd()
        .args(["hook", "claude"])
        .write_stdin(sb.edit_input(&transcript, "src/lib.rs", None))
        .assert()
        .success()
        .stdout("");
}

#[test]
fn init_writes_files_and_check_passes() {
    let sb = Sandbox::new();
    let config = sb.config.join("fresh");
    Command::cargo_bin("modelgate")
        .unwrap()
        .env("HOME", &sb.home)
        .args(["--config-dir", config.to_str().unwrap(), "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote config.toml."));
    assert!(config.join("config.toml").is_file());
    assert!(std::fs::read_to_string(config.join(".gitignore"))
        .unwrap()
        .contains("config.local.toml"));

    sb.cmd()
        .arg("--config-dir")
        .arg(&config)
        .arg("check")
        .assert()
        .success()
        .stdout(predicate::str::contains("Config is valid."));
}

#[test]
fn check_fails_on_bad_placeholder_and_names_file() {
    let sb = Sandbox::new();
    std::fs::write(
        sb.config.join("config.toml"),
        "[[rule]]\nname = \"x\"\nfiles = [\"*.rs\"]\nallowed_models = [\"haiku\"]\nmessage = \"{oops}\"\n",
    )
    .unwrap();
    sb.cmd()
        .arg("check")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("{oops}"));
}

#[test]
fn check_fails_on_invalid_glob() {
    let sb = Sandbox::new();
    std::fs::write(
        sb.config.join("config.toml"),
        "[[rule]]\nname = \"x\"\nfiles = [\"[\"]\nallowed_models = [\"haiku\"]\n",
    )
    .unwrap();
    sb.cmd()
        .arg("check")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("invalid file pattern"));
}

#[test]
fn explain_shows_rule_and_decision() {
    let sb = Sandbox::new();
    sb.cmd()
        .args([
            "explain",
            "src/app.ts",
            "--model",
            "claude-opus-5-5",
            "--cwd",
        ])
        .arg(&sb.project)
        .assert()
        .success()
        .stdout(predicate::str::contains("rule:          source-code"))
        .stdout(predicate::str::contains("decision:      block"));
}

#[test]
fn explain_uses_model_env_when_flag_missing() {
    let sb = Sandbox::new();
    sb.cmd()
        .env("MODELGATE_MODEL", "claude-haiku-5-5")
        .args(["explain", "src/app.ts", "--cwd"])
        .arg(&sb.project)
        .assert()
        .success()
        .stdout(predicate::str::contains("decision:      allow"));
}

#[test]
fn install_prints_snippet_without_touching_settings() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["install", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PreToolUse"))
        .stdout(predicate::str::contains("modelgate hook claude"));
    assert!(!sb.home.join(".claude").exists());
}

#[test]
fn install_write_merges_keeps_hooks_and_is_idempotent() {
    let sb = Sandbox::new();
    let settings_dir = sb.home.join(".claude");
    std::fs::create_dir_all(&settings_dir).unwrap();
    let settings = settings_dir.join("settings.json");
    std::fs::write(
        &settings,
        r#"{"theme":"dark","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"keep-me"}]}]}}"#,
    )
    .unwrap();

    sb.cmd()
        .args(["install", "claude", "--write"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Backup:"));

    let text = std::fs::read_to_string(&settings).unwrap();
    assert!(text.contains("keep-me"));
    assert!(text.contains("modelgate hook claude"));
    assert!(text.contains("\"theme\": \"dark\""));
    let backups = std::fs::read_dir(&settings_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("modelgate-backup"))
        .count();
    assert_eq!(backups, 1);

    sb.cmd()
        .args(["install", "claude", "--write"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already in"));
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), text);
}

#[test]
fn unknown_agent_is_an_error() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["hook", "gemini"])
        .write_stdin("{}")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown agent"));
}
