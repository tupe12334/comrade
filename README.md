# modelgate

modelgate blocks code edits from AI agents unless the editing model is allowed for that file type.

For example, only Haiku may edit `*.rs`. When Opus tries to edit a Rust file, the hook blocks the edit. The agent sees this message: "Send this edit to a Haiku subagent."

modelgate is a Rust library and a command-line tool. Claude Code is supported today.

## Install

```sh
cargo install modelgate
```

Make sure `modelgate` is on your `PATH`. The hook entry runs the command by name.

## Quick start

1. Write the default config:

   ```sh
   modelgate init
   ```

2. Add the hook to Claude Code. This prints the entry:

   ```sh
   modelgate install claude
   ```

   To merge the entry into `~/.claude/settings.json` for you, run:

   ```sh
   modelgate install claude --write
   ```

   This keeps your other hooks. It writes a backup first. Running it again changes nothing.

3. Check that the config is valid:

   ```sh
   modelgate check
   ```

4. See why one file is allowed or blocked:

   ```sh
   modelgate explain src/main.rs --model claude-opus-5-5
   ```

## Commands

| Command | What it does |
| --- | --- |
| `modelgate init [--force]` | Writes `config.toml`, a `.gitignore`, and runs `git init` if needed. Never commits. |
| `modelgate check` | Validates every config file and the merged config. |
| `modelgate explain <file> [--model M] [--cwd DIR]` | Shows the matched rule, the model match, and the decision. |
| `modelgate install claude [--write]` | Prints or writes the `PreToolUse` hook entry. |
| `modelgate hook claude` | Reads the hook input on stdin. Agents call this command, not you. |

All commands accept `--config-dir DIR`.

## Config

The config folder is `~/.config/modelgate`. If `$XDG_CONFIG_HOME` is set, it uses `$XDG_CONFIG_HOME/modelgate`. Set `MODELGATE_CONFIG_DIR` to use another folder.

Config comes in layers. Later layers win:

1. The built-in defaults.
2. `config.toml` in the config folder. Share this file.
3. `config.local.toml` in the config folder. This file is git-ignored. Use it for per-machine values.
4. `.modelgate.toml` in the project root. Use it for one project.

Scalar keys take the value from the last layer that sets them. Rules merge by name. A rule with the same name in a later layer changes only the fields it sets. Rules from later layers are checked first.

The model name comes from the agent transcript. Set `MODELGATE_MODEL` to use a fixed model name instead.

### Default config

This is the file that `modelgate init` writes:

```toml
# modelgate config.
#
# This file lives in ~/.config/modelgate/config.toml. Sync the folder with git
# to share it between computers. Do not put absolute paths or machine-specific
# values here. Put those in config.local.toml, which git ignores.
#
# A project can override this config with a .modelgate.toml file in its root.

# Turns the whole hook on or off.
enabled = true

# Action for files that no rule matches. "allow" or "block".
default_action = "allow"

# Action when the editing model cannot be detected. "allow" or "block".
unknown_model = "block"

# Message sent to the agent when the model is unknown.
# Placeholders: {file}, {model}, {allowed}, {rule}.
unknown_model_message = "Could not detect the model that edits {file}. Send this edit to a Haiku subagent."

# Action when the hook itself fails, for example on bad input or a broken
# config. "allow" keeps a broken config from locking you out.
on_error = "allow"

# Rules. The first enabled rule whose files match decides the edit.
#
# name            Required. Identifies the rule across config layers.
# enabled         Optional. Default true.
# files           Required. Glob patterns. A pattern without "/" matches the
#                 file name. A pattern with "/" matches the path from the
#                 project root.
# allowed_models  Required. Model patterns. A pattern with * or ? is a glob.
#                 Any other pattern is a substring. Both ignore case.
# message         Optional. Placeholders: {file}, {model}, {allowed}, {rule}.
#
# A rule with the same name in a higher layer overrides only the fields it sets.

[[rule]]
name = "source-code"
enabled = true
files = [
    "*.rs",
    "*.ts",
    "*.tsx",
    "*.js",
    "*.jsx",
    "*.py",
    "*.go",
    "*.java",
    "*.kt",
    "*.swift",
    "*.c",
    "*.cpp",
    "*.h",
    "*.cs",
    "*.rb",
    "*.php",
    "*.scala",
    "*.sh",
]
allowed_models = ["*haiku*"]
message = "Send this edit to a Haiku subagent. Model {model} may not edit {file}."

# Example, not active. Uncomment and change to add your own rule.
#
# [[rule]]
# name = "docs-opus-only"
# files = ["docs/**/*.md"]
# allowed_models = ["*opus*"]
# message = "Send this docs edit to an Opus subagent. Model {model} may not edit {file}."
```

### Rule fields

| Field | Required | Meaning |
| --- | --- | --- |
| `name` | yes | Identifies the rule across layers. |
| `enabled` | no | `true` or `false`. Default `true`. |
| `files` | yes | Glob patterns. Patterns without `/` match the file name. Patterns with `/` match the path from the project root. |
| `allowed_models` | yes | Model patterns. A pattern with `*`, `?` or `[` is a glob. Other patterns are substrings. Both ignore case. |
| `message` | no | Text for the agent. Placeholders: `{file}`, `{model}`, `{allowed}`, `{rule}`. |

The first enabled rule whose `files` match decides the edit.

### Other keys

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | Turns the whole hook on or off. |
| `default_action` | `"allow"` | Action for files that no rule matches. |
| `unknown_model` | `"block"` | Action when the model cannot be found. |
| `unknown_model_message` | see default config | Text for the agent when the model is unknown. |
| `on_error` | `"allow"` | Action when the hook fails, for example on bad input or a broken config. |

`"allow"` lets the edit happen. `"block"` stops it.

## Sync the config folder with git

The config folder works as a git repository:

```sh
cd ~/.config/modelgate
git remote add origin <your-private-repo-url>
git add config.toml .gitignore
git commit -m "Update modelgate config"
git push -u origin main
```

On another computer, clone the repository into `~/.config/modelgate`. Then create `config.local.toml` for that computer if you need one.

Keep the folder free of absolute paths and machine-specific values. Put those in `config.local.toml`.

## How the model is found

Claude Code does not send the model in the hook input. modelgate reads the session transcript named in `transcript_path`. It uses the model of the last assistant message.

When the call comes from a subagent, the hook input has an `agent_id`. modelgate then reads the subagent transcript, at `<session>/subagents/agent-<agent_id>.jsonl`. A Haiku subagent can therefore edit files while the main session runs on Opus.

If the subagent transcript is missing, the model is unknown. modelgate does not use the main session model in that case.

## Bash writes

modelgate also checks some Bash commands that write files. It detects:

- output redirects such as `>`, `>>`, `>|` and `&>`
- `tee`
- `sed -i`
- `cp` and `mv`
- `dd of=`

It skips heredoc bodies and `/dev/null`. It does not detect `python -c`, `perl -i`, `awk -i inplace`, `curl -o`, `wget -O`, `git apply`, or commands run through `eval` or `$(...)`. Treat the Bash check as a guard against common cases, not as a full sandbox.

## Fail-safe

If the hook fails, modelgate prints a message to stderr. It then applies `on_error`. The default is `allow`. A broken config does not lock you out. Set `on_error = "block"` if you prefer the opposite.

## Add an agent

Each agent is one module under `src/agents/`. The module implements the `AgentIntegration` trait in `src/agents/mod.rs`:

- `parse` reads the agent hook input and returns the files, the working directory, and where to find the transcript.
- `detect_model` returns the model name, or `None`.
- `block` returns the response that denies the edit.

Then add the agent name to `by_name` in `src/agents/mod.rs`, and add a `hook` and `install` branch in `src/cli.rs` and `src/commands/install.rs`.

Use `src/agents/claude_code/` as the example.

Agents that do not expose the file path and the tool name in their hook input cannot be supported yet.

## Environment variables

| Variable | Meaning |
| --- | --- |
| `MODELGATE_CONFIG_DIR` | Config folder. Overrides the default. |
| `MODELGATE_MODEL` | Model name to use instead of the detected model. |
| `CLAUDE_CONFIG_DIR` | Claude Code settings folder used by `install claude --write`. |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests never read or write your real `~/.config` or `~/.claude`.

## License

Licensed under either of

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.
