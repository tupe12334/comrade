//! Command-line parsing and dispatch.

use std::io::{Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::commands::{check, explain, init, install};
use crate::config::LoadOptions;
use crate::error::Result;
use crate::hook::{self, HookOptions};
use crate::paths;

/// Blocks code edits from AI agents unless the editing model is allowed.
#[derive(Debug, Parser)]
#[command(name = "modelgate", version, about, long_about = None)]
pub struct Cli {
    /// Config folder. Overrides MODELGATE_CONFIG_DIR and the default location.
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write the default config into the config folder and run git init.
    Init {
        /// Overwrite an existing config.toml.
        #[arg(long)]
        force: bool,
    },
    /// Validate the config files and report the first error.
    Check,
    /// Show which rule matches a file and whether the model may edit it.
    Explain {
        /// The file to check. Relative paths use --cwd.
        file: PathBuf,
        /// The model that edits the file. Defaults to MODELGATE_MODEL, else unknown.
        #[arg(long)]
        model: Option<String>,
        /// Working directory for relative paths and the project root. Defaults to the current directory.
        #[arg(long, value_name = "DIR")]
        cwd: Option<PathBuf>,
    },
    /// Print or write the hook entry for an agent.
    Install {
        /// Agent name, for example `claude`.
        agent: String,
        /// Write the entry into the agent settings file. A backup is made first.
        #[arg(long)]
        write: bool,
    },
    /// Run the hook. Reads the agent hook input on stdin. Called by the agent.
    Hook {
        /// Optional agent name, for example `claude`. polyhook detects the agent when omitted.
        agent: Option<String>,
    },
}

/// Runs the command line and returns the process exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    match dispatch(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("modelgate: {error}");
            1
        }
    }
}

fn dispatch(cli: Cli) -> Result<i32> {
    let config_dir = paths::config_dir(cli.config_dir.as_deref())?;
    match cli.command {
        Command::Init { force } => {
            let report = init::init(&config_dir, force)?;
            println!("Config folder: {}", report.dir.display());
            println!(
                "{}",
                if report.wrote_config {
                    "Wrote config.toml."
                } else {
                    "Kept the existing config.toml. Use --force to overwrite it."
                }
            );
            if report.updated_gitignore {
                println!("Updated .gitignore (config.local.toml is ignored).");
            }
            if report.git_initialized {
                println!("Ran git init. Nothing was committed.");
            }
            Ok(0)
        }
        Command::Check => {
            let options = load_options(config_dir, None)?;
            let report = check::check(&options)?;
            print!("{}", check::format_report(&report));
            Ok(0)
        }
        Command::Explain { file, model, cwd } => {
            let cwd = match cwd {
                Some(dir) => dir,
                None => std::env::current_dir().map_err(|source| crate::error::Error::Io {
                    path: PathBuf::from("."),
                    source,
                })?,
            };
            let options = load_options(config_dir, Some(&cwd))?;
            let model = model.or_else(|| std::env::var(paths::MODEL_ENV).ok());
            let text = explain::explain(&options, &file, model.as_deref(), &cwd)?;
            print!("{text}");
            Ok(0)
        }
        Command::Install { agent, write } => {
            if write {
                print!("{}", install::write_instructions(&agent)?);
            } else {
                print!("{}", install::print_instructions(&agent)?);
            }
            Ok(0)
        }
        Command::Hook { agent } => {
            if let Some(name) = agent.as_deref() {
                hook::use_agent_hint(name)?;
            }
            let mut stdin = Vec::new();
            if let Err(error) = std::io::stdin().read_to_end(&mut stdin) {
                stdin.clear();
                eprintln!("modelgate: cannot read stdin: {error}");
            }
            let options = HookOptions {
                config_dir: cli.config_dir,
                model_override: std::env::var(paths::MODEL_ENV)
                    .ok()
                    .filter(|m| !m.trim().is_empty()),
            };
            let outcome = hook::run(&stdin, &options);
            if !outcome.stdout.is_empty() {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{}", outcome.stdout);
            }
            if !outcome.stderr.is_empty() {
                eprintln!("{}", outcome.stderr);
            }
            Ok(outcome.exit_code)
        }
    }
}

fn load_options(config_dir: PathBuf, cwd: Option<&std::path::Path>) -> Result<LoadOptions> {
    let project_root = match cwd {
        Some(dir) => Some(paths::project_root(dir)),
        None => std::env::current_dir()
            .ok()
            .map(|dir| paths::project_root(&dir)),
    };
    Ok(LoadOptions {
        config_dir,
        project_root,
    })
}
