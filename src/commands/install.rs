//! `modelgate install <agent>`: prints or writes the hook entry for an agent.

use std::path::PathBuf;

use crate::agents::claude_code::install::{self, WriteOutcome};
use crate::error::{Error, Result};

/// Returns the text to print without `--write`, and the settings path it targets.
pub fn print_instructions(agent: &str) -> Result<String> {
    match agent {
        "claude" | "claude-code" => {
            let path = install::settings_path()?;
            Ok(format!(
                "Add this entry under \"hooks\" in {}, or run `modelgate install claude --write`.\n\n{}\n",
                path.display(),
                install::snippet()
            ))
        }
        other => Err(Error::UnknownAgent(other.to_string())),
    }
}

/// Writes the hook entry for an agent. Returns a line for the terminal.
pub fn write_instructions(agent: &str) -> Result<String> {
    match agent {
        "claude" | "claude-code" => {
            let path: PathBuf = install::settings_path()?;
            match install::write_settings(&path)? {
                WriteOutcome::Added { path, backup } => Ok(match backup {
                    Some(backup) => format!(
                        "Added the PreToolUse hook to {}.\nBackup: {}\n",
                        path.display(),
                        backup.display()
                    ),
                    None => format!("Created {} with the PreToolUse hook.\n", path.display()),
                }),
                WriteOutcome::AlreadyInstalled { path } => Ok(format!(
                    "The hook is already in {}. Nothing changed.\n",
                    path.display()
                )),
            }
        }
        other => Err(Error::UnknownAgent(other.to_string())),
    }
}
