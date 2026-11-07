//! Locations of the config folder and of project config files.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use crate::error::{Error, Result};

/// Name of the program. Used for the config folder and the project file.
pub const NAME: &str = "comrade";

/// Environment variable that overrides the config folder.
pub const CONFIG_DIR_ENV: &str = "COMRADE_CONFIG_DIR";

/// Environment variable that overrides the editing model.
pub const MODEL_ENV: &str = "COMRADE_MODEL";

/// Shared config file name inside the config folder.
pub const CONFIG_FILE: &str = "config.toml";

/// Per-machine override file inside the config folder. Git-ignored.
pub const LOCAL_CONFIG_FILE: &str = "config.local.toml";

/// Project config file in the project root.
pub const PROJECT_FILE: &str = ".comrade.toml";

/// Returns the config folder.
///
/// Uses `explicit` when given. Otherwise uses `COMRADE_CONFIG_DIR`.
/// Otherwise uses `$XDG_CONFIG_HOME/comrade`, or `~/.config/comrade`.
pub fn config_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let strategy =
        etcetera::choose_base_strategy().map_err(|e| Error::NoConfigDir(e.to_string()))?;
    Ok(strategy.config_dir().join(NAME))
}

/// Finds the project root for a working directory.
///
/// Walks up from `start` and returns the first folder that holds a `.git`
/// entry. If none exists, returns `start` itself.
pub fn project_root(start: &Path) -> PathBuf {
    let mut current = start;
    loop {
        if current.join(".git").exists() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return start.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_dir_wins() {
        let dir = Path::new("/tmp/explicit-comrade");
        assert_eq!(config_dir(Some(dir)).unwrap(), dir);
    }

    #[test]
    fn project_root_finds_git_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let nested = root.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        assert_eq!(project_root(&nested), root);
    }

    #[test]
    fn project_root_falls_back_to_start() {
        let tmp = tempfile::tempdir().unwrap();
        let start = tmp.path().join("plain");
        std::fs::create_dir_all(&start).unwrap();
        // The temp dir itself has no .git above it in normal setups.
        let found = project_root(&start);
        assert!(found.starts_with(tmp.path()) || found == start);
    }
}
