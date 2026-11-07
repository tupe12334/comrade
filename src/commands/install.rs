//! `comrade install <agent>`: prints or writes the hook entry for an agent.
//!
//! Claude Code settings can be written. Other polyhook agents get the
//! instructions to add `comrade hook` by hand.

use std::path::PathBuf;

use crate::agents::{
    self,
    claude_code::install::{self, WriteOutcome},
    AgentInfo,
};
use crate::error::{Error, Result};

/// Returns the text to print without `--write`.
pub fn print_instructions(agent: &str) -> Result<String> {
    let info = known(agent)?;
    if info.name == "claude-code" {
        let path = install::settings_path()?;
        return Ok(format!(
            "Add this entry under \"hooks\" in {}, or run `comrade install claude --write`.\n\n{}\n",
            path.display(),
            install::snippet()
        ));
    }
    Ok(manual_instructions(info))
}

/// Writes the hook entry for an agent. Returns a line for the terminal.
pub fn write_instructions(agent: &str) -> Result<String> {
    let info = known(agent)?;
    if info.name != "claude-code" {
        return Err(Error::WriteNotSupported(agent.to_string()));
    }
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

fn known(agent: &str) -> Result<&'static AgentInfo> {
    agents::find(agent).ok_or_else(|| Error::UnknownAgent(agent.to_string()))
}

fn manual_instructions(info: &AgentInfo) -> String {
    format!(
        "Register `comrade hook` as the command for the `{event}` hook in {name}.\n\
         The hook reads the agent input on stdin and writes the response to stdout.\n\
         Set COMRADE_MODEL to the model name. {name} has no model lookup yet.\n\
         Hook docs: {docs}\n",
        event = info.pre_tool_event,
        name = info.name,
        docs = info.docs,
    )
}
