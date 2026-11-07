//! `modelgate init`: writes the default config into the config folder.

use std::path::Path;
use std::process::Command;

use crate::config::DEFAULT_CONFIG_TEXT;
use crate::error::{Error, Result};
use crate::paths::{CONFIG_FILE, LOCAL_CONFIG_FILE};

/// What `init` did. Each flag tells whether that step ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// The config folder.
    pub dir: std::path::PathBuf,
    /// `config.toml` was written.
    pub wrote_config: bool,
    /// `.gitignore` was created or updated.
    pub updated_gitignore: bool,
    /// `git init` ran.
    pub git_initialized: bool,
}

/// Writes the default config, a `.gitignore`, and runs `git init` if needed.
///
/// Never overwrites `config.toml` unless `force` is set. Never commits.
pub fn init(dir: &Path, force: bool) -> Result<InitReport> {
    std::fs::create_dir_all(dir).map_err(|source| io(dir, source))?;

    let config_path = dir.join(CONFIG_FILE);
    let wrote_config = if config_path.exists() && !force {
        false
    } else {
        std::fs::write(&config_path, DEFAULT_CONFIG_TEXT)
            .map_err(|source| io(&config_path, source))?;
        true
    };

    let updated_gitignore = update_gitignore(dir)?;

    let git_initialized = if dir.join(".git").exists() || !git_available() {
        false
    } else {
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(dir)
            .status()
            .map_err(|source| io(dir, source))?
            .success()
    };

    Ok(InitReport {
        dir: dir.to_path_buf(),
        wrote_config,
        updated_gitignore,
        git_initialized,
    })
}

fn update_gitignore(dir: &Path) -> Result<bool> {
    let path = dir.join(".gitignore");
    let existing = if path.exists() {
        std::fs::read_to_string(&path).map_err(|source| io(&path, source))?
    } else {
        String::new()
    };
    if existing
        .lines()
        .any(|line| line.trim() == LOCAL_CONFIG_FILE)
    {
        return Ok(false);
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(LOCAL_CONFIG_FILE);
    text.push('\n');
    std::fs::write(&path, text).map_err(|source| io(&path, source))?;
    Ok(true)
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_config_and_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let report = init(tmp.path(), false).unwrap();
        assert!(report.wrote_config);
        assert!(report.updated_gitignore);
        let text = std::fs::read_to_string(tmp.path().join(CONFIG_FILE)).unwrap();
        assert_eq!(text, DEFAULT_CONFIG_TEXT);
        let ignore = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == LOCAL_CONFIG_FILE));
    }

    #[test]
    fn keeps_existing_config_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(CONFIG_FILE), "enabled = false\n").unwrap();
        let report = init(tmp.path(), false).unwrap();
        assert!(!report.wrote_config);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(CONFIG_FILE)).unwrap(),
            "enabled = false\n"
        );
    }

    #[test]
    fn force_overwrites_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(CONFIG_FILE), "enabled = false\n").unwrap();
        assert!(init(tmp.path(), true).unwrap().wrote_config);
    }

    #[test]
    fn gitignore_is_not_duplicated() {
        let tmp = tempfile::tempdir().unwrap();
        init(tmp.path(), false).unwrap();
        let second = init(tmp.path(), false).unwrap();
        assert!(!second.updated_gitignore);
        let ignore = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(ignore.matches(LOCAL_CONFIG_FILE).count(), 1);
    }

    #[test]
    fn runs_git_init_when_git_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let report = init(tmp.path(), false).unwrap();
        if git_available() {
            assert!(report.git_initialized);
            assert!(tmp.path().join(".git").exists());
        } else {
            assert!(!report.git_initialized);
        }
    }
}
