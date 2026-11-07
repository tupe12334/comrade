//! Loads config layers from disk and merges them.

use std::path::{Path, PathBuf};

use crate::config::{
    validate, Action, Config, RawConfig, RuleSpec, DEFAULT_CONFIG_TEXT, DEFAULT_RULE_MESSAGE,
};
use crate::error::{Error, Result};
use crate::paths::{CONFIG_FILE, LOCAL_CONFIG_FILE, PROJECT_FILE};

/// Message used when the config does not set `unknown_model_message`.
const FALLBACK_UNKNOWN_MODEL_MESSAGE: &str =
    "Could not detect the model that edits {file}. Send this edit to a Haiku subagent.";

/// One parsed config layer.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Where the layer came from. A path, or `built-in defaults`.
    pub source: String,
    /// The parsed content.
    pub raw: RawConfig,
}

/// Where to look for config files.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// The config folder (`~/.config/modelgate` by default).
    pub config_dir: PathBuf,
    /// The project root. `.modelgate.toml` is read from here when set.
    pub project_root: Option<PathBuf>,
}

/// Loads every layer, lowest precedence first, and validates each file.
pub fn load_layers(options: &LoadOptions) -> Result<Vec<Layer>> {
    let mut layers = vec![Layer {
        source: "built-in defaults".to_string(),
        raw: parse_text(DEFAULT_CONFIG_TEXT, "built-in defaults")?,
    }];

    let mut paths = vec![
        options.config_dir.join(CONFIG_FILE),
        options.config_dir.join(LOCAL_CONFIG_FILE),
    ];
    if let Some(root) = &options.project_root {
        paths.push(root.join(PROJECT_FILE));
    }

    for path in paths {
        if let Some(layer) = read_layer(&path)? {
            layers.push(layer);
        }
    }
    Ok(layers)
}

/// Loads and merges every layer into one config.
pub fn load(options: &LoadOptions) -> Result<Config> {
    merge(&load_layers(options)?)
}

fn read_layer(path: &Path) -> Result<Option<Layer>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let source = path.display().to_string();
    let raw = parse_text(&text, &source)?;
    validate::unique_rule_names(&raw, &source)?;
    Ok(Some(Layer { source, raw }))
}

fn parse_text(text: &str, source: &str) -> Result<RawConfig> {
    toml::from_str(text).map_err(|source_error| Error::Toml {
        source_name: source.to_string(),
        source: source_error,
    })
}

/// Merges layers (lowest precedence first) into one config.
///
/// Rules merge by name. Rules first defined in a higher layer come first in
/// the result, so they are checked first.
pub fn merge(layers: &[Layer]) -> Result<Config> {
    let mut enabled = None;
    let mut default_action = None;
    let mut unknown_model = None;
    let mut unknown_model_message = None;
    let mut on_error = None;
    let mut partials: Vec<(usize, Partial)> = Vec::new();

    for (rank, layer) in layers.iter().enumerate() {
        enabled = layer.raw.enabled.or(enabled);
        default_action = layer.raw.default_action.or(default_action);
        unknown_model = layer.raw.unknown_model.or(unknown_model);
        on_error = layer.raw.on_error.or(on_error);
        if let Some(message) = &layer.raw.unknown_model_message {
            unknown_model_message = Some(message.clone());
        }

        for raw in &layer.raw.rules {
            match partials.iter_mut().find(|(_, p)| p.name == raw.name) {
                Some((_, existing)) => existing.apply(raw),
                None => partials.push((rank, Partial::from_raw(raw, &layer.source))),
            }
        }
    }

    // Higher rank (defined in a higher layer) first. The sort is stable, so
    // rules keep their order within one layer.
    partials.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));

    let rules = partials
        .into_iter()
        .map(|(_, partial)| partial.finish())
        .collect::<Result<Vec<RuleSpec>>>()?;

    Ok(Config {
        enabled: enabled.unwrap_or(true),
        default_action: default_action.unwrap_or(Action::Allow),
        unknown_model: unknown_model.unwrap_or(Action::Block),
        unknown_model_message: unknown_model_message
            .unwrap_or_else(|| FALLBACK_UNKNOWN_MODEL_MESSAGE.to_string()),
        on_error: on_error.unwrap_or(Action::Allow),
        rules,
    })
}

/// A rule that may still miss required fields.
struct Partial {
    name: String,
    source: String,
    enabled: Option<bool>,
    files: Option<Vec<String>>,
    allowed_models: Option<Vec<String>>,
    message: Option<String>,
}

impl Partial {
    fn from_raw(raw: &crate::config::RawRule, source: &str) -> Self {
        Self {
            name: raw.name.clone(),
            source: source.to_string(),
            enabled: raw.enabled,
            files: raw.files.clone(),
            allowed_models: raw.allowed_models.clone(),
            message: raw.message.clone(),
        }
    }

    fn apply(&mut self, raw: &crate::config::RawRule) {
        self.enabled = raw.enabled.or(self.enabled);
        if raw.files.is_some() {
            self.files = raw.files.clone();
        }
        if raw.allowed_models.is_some() {
            self.allowed_models = raw.allowed_models.clone();
        }
        if raw.message.is_some() {
            self.message = raw.message.clone();
        }
    }

    fn finish(self) -> Result<RuleSpec> {
        let fail = |message: String| Error::Config {
            source_name: self.source.clone(),
            message,
        };
        let files = self
            .files
            .ok_or_else(|| fail(format!("rule `{}` has no `files`", self.name)))?;
        if files.is_empty() {
            return Err(fail(format!(
                "rule `{}` has an empty `files` list",
                self.name
            )));
        }
        let allowed_models = self
            .allowed_models
            .ok_or_else(|| fail(format!("rule `{}` has no `allowed_models`", self.name)))?;
        Ok(RuleSpec {
            name: self.name,
            enabled: self.enabled.unwrap_or(true),
            files,
            allowed_models,
            message: self
                .message
                .unwrap_or_else(|| DEFAULT_RULE_MESSAGE.to_string()),
            source: self.source,
        })
    }
}
