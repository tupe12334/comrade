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

/// True when `file` may skip the rules as a file outside any git repository.
///
/// Fails closed: returns false unless every check passes. The file must be
/// absolute and outside `/proc`, `/dev` and `/sys`. Its parent folder must
/// exist. Neither the path as written nor the canonical path may have a `.git`
/// ancestor. An existing file that is not a folder must have one hard link
/// (unix only). `..` is resolved by the OS through `canonicalize`, after
/// symlinks, so no text rewriting is needed.
pub fn outside_any_repo(file: &Path) -> bool {
    if !file.is_absolute()
        || ["/proc", "/dev", "/sys"]
            .iter()
            .any(|p| file.starts_with(p))
    {
        return false;
    }
    let (Some(name), Some(parent)) = (file.file_name(), file.parent()) else {
        return false;
    };
    let Ok(real_parent) = parent.canonicalize() else {
        return false;
    };
    let joined = real_parent.join(name);
    if has_git_ancestor(file) || has_git_ancestor(&joined) {
        return false;
    }
    match std::fs::metadata(file) {
        Ok(meta) => {
            let Ok(real_file) = file.canonicalize() else {
                return false;
            };
            if has_git_ancestor(&real_file) {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if !meta.is_dir() && meta.nlink() > 1 {
                    return false;
                }
            }
        }
        // A dangling symlink exists but has no target. Keep the rules for it.
        Err(_) if file.symlink_metadata().is_ok() => return false,
        Err(_) => {}
    }
    true
}

/// Walks up from the parent folder, so the file need not exist yet.
fn has_git_ancestor(file: &Path) -> bool {
    let mut current = file.parent();
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return true;
        }
        current = dir.parent();
    }
    false
}

/// Returns a temp folder that has no `.git` above it, or prints a skip note.
///
/// Tests that assert "outside a repo" need this. On a machine where the temp
/// folder sits inside a repo, those tests skip, and the note says so.
#[cfg(test)]
pub(crate) fn git_free_tempdir() -> Option<tempfile::TempDir> {
    let tmp = tempfile::tempdir().unwrap();
    if has_git_ancestor(&tmp.path().join("probe")) {
        eprintln!(
            "SKIPPED: a .git sits above {}. Outside-repo assertions did not run.",
            tmp.path().display()
        );
        return None;
    }
    Some(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_in_nested_git_repo_is_not_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        assert!(!outside_any_repo(&root.join("src/deep/step.rs")));
    }

    #[test]
    fn git_file_counts_as_repo() {
        // A worktree or submodule holds a `.git` file, not a folder.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("worktree");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join(".git"), "gitdir: /elsewhere/.git/worktrees/w\n").unwrap();
        assert!(!outside_any_repo(&root.join("src/step.rs")));
    }

    #[test]
    fn plain_file_in_git_free_folder_is_outside() {
        let Some(tmp) = git_free_tempdir() else {
            return;
        };
        let plain = tmp.path().join("scratch");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(outside_any_repo(&plain.join("step.sh")));
    }

    #[test]
    fn missing_parent_folder_is_not_outside() {
        let Some(tmp) = git_free_tempdir() else {
            return;
        };
        assert!(!outside_any_repo(&tmp.path().join("new/dir/x.rs")));
    }

    #[test]
    fn relative_path_is_not_outside() {
        assert!(!outside_any_repo(Path::new("step.sh")));
    }

    #[test]
    fn proc_dev_and_sys_are_not_outside() {
        assert!(!outside_any_repo(Path::new("/proc/self/cwd/x.rs")));
        assert!(!outside_any_repo(Path::new("/dev/null")));
        assert!(!outside_any_repo(Path::new("/sys/kernel/x")));
    }

    #[test]
    fn dotdot_through_symlink_resolves_to_real_parent() {
        // `lnk/..` is the repo root, not the temp folder, so the file is in a repo.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        #[cfg(unix)]
        {
            let lnk = tmp.path().join("lnk");
            std::os::unix::fs::symlink(root.join("src"), &lnk).unwrap();
            assert!(!outside_any_repo(&lnk.join("../a.rs")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn dir_symlink_into_repo_is_not_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(root.join("src"), &link).unwrap();
        assert!(!outside_any_repo(&link.join("step.rs")));
    }

    #[cfg(unix)]
    #[test]
    fn file_symlink_into_repo_is_not_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("x.rs"), "").unwrap();
        let link = tmp.path().join("a.rs");
        std::os::unix::fs::symlink(root.join("x.rs"), &link).unwrap();
        assert!(!outside_any_repo(&link));
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_to_repo_file_is_not_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("src/h.rs"), "x").unwrap();
        let hl = tmp.path().join("hl.rs");
        std::fs::hard_link(root.join("src/h.rs"), &hl).unwrap();
        assert!(!outside_any_repo(&hl));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_not_outside() {
        let Some(tmp) = git_free_tempdir() else {
            return;
        };
        let link = tmp.path().join("dangling.rs");
        std::os::unix::fs::symlink(tmp.path().join("missing/x.rs"), &link).unwrap();
        assert!(!outside_any_repo(&link));
    }

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
