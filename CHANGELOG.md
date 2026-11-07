# Changelog

All notable changes to this project are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Config layers: built-in defaults, `config.toml`, `config.local.toml`, and `.modelgate.toml`.
- Rules with glob file patterns and model patterns (substring or glob, case-insensitive).
- Commands: `init`, `check`, `explain`, `install claude`, and `hook claude`.
- Claude Code `PreToolUse` hook for `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, and simple Bash file writes.
- Model detection from the session transcript and from subagent transcripts.
- `on_error` and `unknown_model` policies.
