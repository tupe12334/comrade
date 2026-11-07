//! Config types and the loading entry point.
//!
//! Config comes in layers, lowest precedence first:
//!
//! 1. The built-in defaults (`src/default_config.toml`).
//! 2. `<config dir>/config.toml`, the shared file you sync between computers.
//! 3. `<config dir>/config.local.toml`, a per-machine override.
//! 4. `.modelgate.toml` in the project root.
//!
//! Scalar keys take the value from the highest layer that sets them. Rules
//! merge by name: a rule in a higher layer overrides only the fields it sets.
//! New rules from higher layers are checked first.

pub mod layers;
pub mod validate;

use serde::{Deserialize, Serialize};

pub use layers::{load, load_layers, Layer, LoadOptions};

/// The built-in default config. `modelgate init` writes this text.
pub const DEFAULT_CONFIG_TEXT: &str = include_str!("default_config.toml");

/// What to do with an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Let the edit happen.
    Allow,
    /// Stop the edit and tell the agent why.
    Block,
}

/// One layer of config exactly as written in a TOML file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    /// Turns the whole hook on or off.
    pub enabled: Option<bool>,
    /// Action for files that no rule matches.
    pub default_action: Option<Action>,
    /// Action when the editing model cannot be detected.
    pub unknown_model: Option<Action>,
    /// Message used when the model is unknown.
    pub unknown_model_message: Option<String>,
    /// Action when the hook itself fails.
    pub on_error: Option<Action>,
    /// The `[[rule]]` tables.
    #[serde(default, rename = "rule")]
    pub rules: Vec<RawRule>,
}

/// One `[[rule]]` table. Every field except `name` is optional, so a rule can
/// override only part of a rule from a lower layer.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawRule {
    /// Identifies the rule across layers.
    pub name: String,
    /// Turns this rule on or off.
    pub enabled: Option<bool>,
    /// Glob patterns for the files this rule covers.
    pub files: Option<Vec<String>>,
    /// Model patterns that may edit the files. Substring or glob, case-insensitive.
    pub allowed_models: Option<Vec<String>>,
    /// Message template sent to the agent when the edit is blocked.
    pub message: Option<String>,
}

/// A rule after all layers were merged. Not yet compiled into matchers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSpec {
    /// Rule name.
    pub name: String,
    /// Whether the rule is active.
    pub enabled: bool,
    /// Glob patterns for covered files.
    pub files: Vec<String>,
    /// Allowed model patterns.
    pub allowed_models: Vec<String>,
    /// Message template for a blocked edit.
    pub message: String,
    /// The layer that first defined this rule.
    pub source: String,
}

/// Fully merged config, ready to compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Turns the whole hook on or off.
    pub enabled: bool,
    /// Action for files that no rule matches.
    pub default_action: Action,
    /// Action when the editing model is unknown.
    pub unknown_model: Action,
    /// Message used when the model is unknown.
    pub unknown_model_message: String,
    /// Action when the hook itself fails.
    pub on_error: Action,
    /// Rules, highest precedence first.
    pub rules: Vec<RuleSpec>,
}

/// Message used when a rule does not set one.
pub const DEFAULT_RULE_MESSAGE: &str =
    "Send this edit to a Haiku subagent. Model {model} may not edit {file}.";

impl Config {
    /// Returns the config made from the built-in defaults only.
    pub fn defaults() -> Self {
        let raw: RawConfig =
            toml::from_str(DEFAULT_CONFIG_TEXT).expect("built-in default config must parse");
        layers::merge(&[Layer {
            source: "built-in defaults".to_string(),
            raw,
        }])
        .expect("built-in default config must be valid")
    }
}
