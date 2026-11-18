# Changelog

All notable changes to this project are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.0](https://github.com/tupe12334/comrade/compare/v0.1.0...v0.2.0) (2026-11-18)


### Features

* add cargo-deny dependency check to CI ([#13](https://github.com/tupe12334/comrade/issues/13)) ([1ba06b4](https://github.com/tupe12334/comrade/commit/1ba06b47f2d89adad61c57cdb1ce7ab4ea50357e))
* add release-please changelog and PR title check ([#14](https://github.com/tupe12334/comrade/issues/14)) ([eb3d2a2](https://github.com/tupe12334/comrade/commit/eb3d2a25d3567ad3b252e035533c9217ba21f313))

## [Unreleased]

## [0.1.0] - 2026-11-07

### Changed

- Renamed from modelgate.
- Hook input and output now go through the `polyhook` crate. comrade supports every agent that polyhook supports.
- `comrade hook` works without an agent name. `comrade hook claude` still works.
- `comrade install claude` writes `comrade hook`. Existing `comrade hook claude` entries still work.
- `comrade install <agent>` prints the hook event and docs link for each polyhook agent. `--write` still edits Claude Code settings only.
- The `COMRADE_MODEL` override applies to every agent.
- Edit tools from other agents (`write_file`, `edit_file`) and Bash writes are checked the same way as Claude Code calls.

### Removed

- The Claude Code input and output modules. polyhook replaces them.
- The `AgentIntegration` trait.

### Added

- Supported agents section in the README, listing the agents that polyhook supports.
- Config layers: built-in defaults, `config.toml`, `config.local.toml`, and `.comrade.toml`.
- Rules with glob file patterns and model patterns (substring or glob, case-insensitive).
- Commands: `init`, `check`, `explain`, `install claude`, and `hook claude`.
- Claude Code `PreToolUse` hook for `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, and simple Bash file writes.
- Model detection from the session transcript and from subagent transcripts.
- `on_error` and `unknown_model` policies.

### Fixed

- Subagent model falls back to `agent-<agent_id>.meta.json` when the subagent transcript is not flushed yet.
