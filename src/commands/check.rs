//! `modelgate check`: validates every config layer and the merged config.

use crate::config::{self, validate, LoadOptions};
use crate::error::Result;
use crate::rules::Engine;

/// Result of a successful check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// Layers that were read, lowest precedence first.
    pub sources: Vec<String>,
    /// Number of rules after merging.
    pub rules: usize,
    /// Number of rules that are enabled.
    pub enabled_rules: usize,
}

/// Loads, merges, and compiles the config. Fails with a message that names the file.
pub fn check(options: &LoadOptions) -> Result<CheckReport> {
    let layers = config::load_layers(options)?;
    let merged = config::layers::merge(&layers)?;
    validate::check_config(&merged)?;
    Engine::new(&merged)?;
    Ok(CheckReport {
        sources: layers.iter().map(|l| l.source.clone()).collect(),
        rules: merged.rules.len(),
        enabled_rules: merged.rules.iter().filter(|r| r.enabled).count(),
    })
}

/// Formats a successful report for the terminal.
pub fn format_report(report: &CheckReport) -> String {
    let mut out = String::from("Config is valid.\n");
    for source in &report.sources {
        out.push_str(&format!("  layer: {source}\n"));
    }
    out.push_str(&format!(
        "  rules: {} ({} enabled)\n",
        report.rules, report.enabled_rules
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_checks_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let options = LoadOptions {
            config_dir: tmp.path().to_path_buf(),
            project_root: None,
        };
        let report = check(&options).unwrap();
        assert_eq!(report.rules, 1);
        assert_eq!(report.enabled_rules, 1);
        assert_eq!(report.sources[0], "built-in defaults");
    }

    #[test]
    fn broken_file_names_its_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "enabled = \"yes\"\n").unwrap();
        let options = LoadOptions {
            config_dir: tmp.path().to_path_buf(),
            project_root: None,
        };
        let err = check(&options).unwrap_err().to_string();
        assert!(err.contains("config.toml"), "{err}");
    }

    #[test]
    fn unknown_key_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "enbled = true\n").unwrap();
        let options = LoadOptions {
            config_dir: tmp.path().to_path_buf(),
            project_root: None,
        };
        assert!(check(&options).is_err());
    }
}
