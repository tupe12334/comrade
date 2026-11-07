//! Checks that do not need the rule matchers. Glob syntax is checked when the
//! engine compiles the rules.

use std::collections::HashSet;

use crate::config::{Config, RawConfig};
use crate::error::{Error, Result};
use crate::template;

/// Fails when one file defines the same rule name twice.
pub fn unique_rule_names(raw: &RawConfig, source: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for rule in &raw.rules {
        if rule.name.trim().is_empty() {
            return Err(Error::Config {
                source_name: source.to_string(),
                message: "a rule has an empty `name`".to_string(),
            });
        }
        if !seen.insert(rule.name.as_str()) {
            return Err(Error::Config {
                source_name: source.to_string(),
                message: format!("rule `{}` is defined twice in this file", rule.name),
            });
        }
    }
    Ok(())
}

/// Fails when a message uses a placeholder that does not exist.
pub fn check_config(config: &Config) -> Result<()> {
    let source = "merged config";
    let mut messages = vec![(
        "unknown_model_message".to_string(),
        config.unknown_model_message.clone(),
    )];
    for rule in &config.rules {
        if rule.name.trim().is_empty() {
            return Err(Error::Config {
                source_name: rule.source.clone(),
                message: "a rule has an empty `name`".to_string(),
            });
        }
        messages.push((
            format!("message of rule `{}`", rule.name),
            rule.message.clone(),
        ));
    }
    for (what, text) in messages {
        if let Some(name) = template::unknown_placeholders(&text).first() {
            return Err(Error::Config {
                source_name: source.to_string(),
                message: format!("{what} uses unknown placeholder `{{{name}}}`"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::layers::{merge, Layer};

    fn raw(text: &str) -> RawConfig {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn duplicate_names_in_one_file_fail() {
        let r = raw(r#"
            [[rule]]
            name = "a"
            files = ["*.rs"]
            allowed_models = ["*haiku*"]

            [[rule]]
            name = "a"
            files = ["*.ts"]
            allowed_models = ["*haiku*"]
            "#);
        let err = unique_rule_names(&r, "x.toml").unwrap_err();
        assert!(err.to_string().contains("defined twice"));
    }

    #[test]
    fn unknown_placeholder_in_message_fails() {
        let layer = Layer {
            source: "t".into(),
            raw: raw(r#"
                [[rule]]
                name = "a"
                files = ["*.rs"]
                allowed_models = ["*haiku*"]
                message = "nope {colour}"
                "#),
        };
        let config = merge(&[layer]).unwrap();
        let err = check_config(&config).unwrap_err();
        assert!(err.to_string().contains("{colour}"));
    }

    #[test]
    fn defaults_pass_validation() {
        assert!(check_config(&Config::defaults()).is_ok());
    }
}
