//! Compiles rules into matchers and decides whether an edit is allowed.

use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};

use crate::config::{Action, Config, RuleSpec};
use crate::error::{Error, Result};
use crate::template;

/// Message used when `default_action` blocks a file that no rule covers.
const NO_RULE_BLOCK_MESSAGE: &str =
    "Comrade {model}, the Party does not type. Send a Haiku comrade (subagent) to edit {file}.";

/// The outcome for one edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The edit may happen.
    Allow,
    /// The edit must not happen. The message goes to the agent.
    Block {
        /// Text for the agent.
        message: String,
    },
}

/// Full explanation of one decision. `comrade explain` prints this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    /// The file, relative to the project root when it is inside it.
    pub file: String,
    /// The model that was checked, if known.
    pub model: Option<String>,
    /// The first rule whose files matched, if any.
    pub rule: Option<String>,
    /// The allowed model pattern that matched, if any.
    pub model_pattern: Option<String>,
    /// A short sentence that says why.
    pub reason: String,
    /// The decision.
    pub verdict: Verdict,
}

/// Compiled config. Build once, evaluate many edits.
#[derive(Debug, Clone)]
pub struct Engine {
    enabled: bool,
    default_action: Action,
    unknown_model: Action,
    unknown_model_message: String,
    rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
struct CompiledRule {
    name: String,
    enabled: bool,
    files: Vec<FilePattern>,
    allowed: Vec<String>,
    models: Vec<ModelPattern>,
    message: String,
}

#[derive(Debug, Clone)]
struct FilePattern {
    matcher: GlobMatcher,
    /// True when the pattern has no `/`, so it matches the file name only.
    file_name_only: bool,
}

#[derive(Debug, Clone)]
enum ModelPattern {
    /// Case-insensitive substring match.
    Substring(String),
    /// Case-insensitive glob match.
    Glob(GlobMatcher),
}

impl Engine {
    /// Compiles a merged config. Fails on an invalid glob.
    pub fn new(config: &Config) -> Result<Self> {
        let rules = config
            .rules
            .iter()
            .map(compile_rule)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            enabled: config.enabled,
            default_action: config.default_action,
            unknown_model: config.unknown_model,
            unknown_model_message: config.unknown_model_message.clone(),
            rules,
        })
    }

    /// Decides one edit.
    ///
    /// `root` is the project root. Patterns with `/` match the path relative to
    /// it. `model` is `None` when the model is unknown.
    pub fn evaluate(&self, file: &Path, root: Option<&Path>, model: Option<&str>) -> Evaluation {
        let display = display_path(file, root);
        let model = model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string);

        if !self.enabled {
            return self.finish(
                display,
                model,
                None,
                None,
                "comrade is disabled",
                Verdict::Allow,
            );
        }

        let Some(rule) = self
            .rules
            .iter()
            .find(|r| r.enabled && r.covers(file, &display))
        else {
            let (verdict, reason) = match self.default_action {
                Action::Allow => (
                    Verdict::Allow,
                    "no rule matches; default_action is allow".to_string(),
                ),
                Action::Block => (
                    Verdict::Block {
                        message: template::render(
                            NO_RULE_BLOCK_MESSAGE,
                            &[
                                ("file", &display),
                                ("model", model.as_deref().unwrap_or("unknown")),
                            ],
                        ),
                    },
                    "no rule matches; default_action is block".to_string(),
                ),
            };
            return self.finish(display, model, None, None, &reason, verdict);
        };

        let Some(model_name) = model.clone() else {
            return match self.unknown_model {
                Action::Allow => self.finish(
                    display.clone(),
                    None,
                    Some(rule.name.clone()),
                    None,
                    "model is unknown; unknown_model is allow",
                    Verdict::Allow,
                ),
                Action::Block => {
                    let message = render_rule(
                        &self.unknown_model_message,
                        &display,
                        "unknown",
                        &rule.allowed.join(", "),
                        &rule.name,
                    );
                    self.finish(
                        display,
                        None,
                        Some(rule.name.clone()),
                        None,
                        "model is unknown; unknown_model is block",
                        Verdict::Block { message },
                    )
                }
            };
        };

        if let Some(index) = rule.models.iter().position(|p| p.matches(&model_name)) {
            let pattern = rule.allowed[index].clone();
            let reason = format!("rule `{}` allows model via `{pattern}`", rule.name);
            return self.finish(
                display,
                model,
                Some(rule.name.clone()),
                Some(pattern),
                &reason,
                Verdict::Allow,
            );
        }

        let message = render_rule(
            &rule.message,
            &display,
            &model_name,
            &rule.allowed.join(", "),
            &rule.name,
        );
        let reason = format!("rule `{}` does not allow model `{model_name}`", rule.name);
        self.finish(
            display,
            model,
            Some(rule.name.clone()),
            None,
            &reason,
            Verdict::Block { message },
        )
    }

    fn finish(
        &self,
        file: String,
        model: Option<String>,
        rule: Option<String>,
        model_pattern: Option<String>,
        reason: &str,
        verdict: Verdict,
    ) -> Evaluation {
        Evaluation {
            file,
            model,
            rule,
            model_pattern,
            reason: reason.to_string(),
            verdict,
        }
    }
}

impl CompiledRule {
    fn covers(&self, file: &Path, display: &str) -> bool {
        let file_name = file
            .file_name()
            .map(|n| n.to_string_lossy().replace('\\', "/"));
        self.files.iter().any(|pattern| {
            if pattern.file_name_only {
                file_name
                    .as_deref()
                    .is_some_and(|n| pattern.matcher.is_match(n))
            } else {
                pattern.matcher.is_match(display)
            }
        })
    }
}

impl ModelPattern {
    fn matches(&self, model: &str) -> bool {
        match self {
            Self::Substring(needle) => model.to_lowercase().contains(needle),
            Self::Glob(matcher) => matcher.is_match(model),
        }
    }
}

fn compile_rule(spec: &RuleSpec) -> Result<CompiledRule> {
    let fail = |message: String| Error::Config {
        source_name: spec.source.clone(),
        message: format!("rule `{}`: {message}", spec.name),
    };

    let files = spec
        .files
        .iter()
        .map(|pattern| {
            let matcher = GlobBuilder::new(pattern)
                .build()
                .map_err(|e| fail(format!("invalid file pattern `{pattern}`: {e}")))?
                .compile_matcher();
            Ok(FilePattern {
                matcher,
                file_name_only: !pattern.contains('/'),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let models = spec
        .allowed_models
        .iter()
        .map(|pattern| {
            if pattern.contains(['*', '?', '[']) {
                let matcher = GlobBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .map_err(|e| fail(format!("invalid model pattern `{pattern}`: {e}")))?
                    .compile_matcher();
                Ok(ModelPattern::Glob(matcher))
            } else {
                Ok(ModelPattern::Substring(pattern.to_lowercase()))
            }
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(CompiledRule {
        name: spec.name.clone(),
        enabled: spec.enabled,
        files,
        allowed: spec.allowed_models.clone(),
        models,
        message: spec.message.clone(),
    })
}

fn render_rule(template_text: &str, file: &str, model: &str, allowed: &str, rule: &str) -> String {
    template::render(
        template_text,
        &[
            ("file", file),
            ("model", model),
            ("allowed", allowed),
            ("rule", rule),
        ],
    )
}

/// Returns the path as text. Relative to `root` when inside it, with `/` separators.
pub fn display_path(file: &Path, root: Option<&Path>) -> String {
    let relative = root.and_then(|r| file.strip_prefix(r).ok()).unwrap_or(file);
    relative.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::layers::{merge, Layer};
    use crate::config::Config;
    use crate::config::RawConfig;
    use std::path::PathBuf;

    fn engine_from(toml_text: &str) -> Engine {
        let raw: RawConfig = toml::from_str(toml_text).unwrap();
        let config = merge(&[
            Layer {
                source: "built-in defaults".into(),
                raw: toml::from_str(crate::config::DEFAULT_CONFIG_TEXT).unwrap(),
            },
            Layer {
                source: "test".into(),
                raw,
            },
        ])
        .unwrap();
        Engine::new(&config).unwrap()
    }

    fn default_engine() -> Engine {
        Engine::new(&Config::defaults()).unwrap()
    }

    fn blocked(v: &Verdict) -> bool {
        matches!(v, Verdict::Block { .. })
    }

    #[test]
    fn haiku_may_edit_rust_and_opus_may_not() {
        let engine = default_engine();
        let file = Path::new("/proj/src/lib.rs");
        assert_eq!(
            engine
                .evaluate(file, None, Some("claude-haiku-5-5"))
                .verdict,
            Verdict::Allow
        );
        let opus = engine.evaluate(file, None, Some("claude-opus-5-5"));
        assert!(blocked(&opus.verdict));
        assert_eq!(opus.rule.as_deref(), Some("source-code"));
    }

    #[test]
    fn block_message_names_model_and_file() {
        let engine = default_engine();
        let eval = engine.evaluate(Path::new("/proj/a.ts"), None, Some("opus"));
        match eval.verdict {
            Verdict::Block { message } => {
                assert_eq!(
                    message,
                    "Comrade opus, the Party does not type. Send a Haiku comrade (subagent) to edit /proj/a.ts."
                );
            }
            Verdict::Allow => panic!("expected block"),
        }
    }

    #[test]
    fn docs_are_not_blocked_by_default() {
        let engine = default_engine();
        for file in [
            "/p/README.md",
            "/p/package.json",
            "/p/Cargo.toml",
            "/p/ci.yaml",
        ] {
            let eval = engine.evaluate(Path::new(file), None, Some("opus"));
            assert_eq!(eval.verdict, Verdict::Allow, "{file}");
            assert!(eval.rule.is_none());
        }
    }

    #[test]
    fn unknown_model_blocks_by_default() {
        let engine = default_engine();
        let eval = engine.evaluate(Path::new("/p/a.py"), None, None);
        assert!(blocked(&eval.verdict));
        match eval.verdict {
            Verdict::Block { message } => {
                assert!(message.contains("the Party could not tell who typed /p/a.py"))
            }
            Verdict::Allow => unreachable!(),
        }
    }

    #[test]
    fn blank_model_counts_as_unknown() {
        let engine = default_engine();
        assert!(blocked(
            &engine
                .evaluate(Path::new("/p/a.py"), None, Some("  "))
                .verdict
        ));
    }

    #[test]
    fn unknown_model_can_allow() {
        let engine = engine_from("unknown_model = \"allow\"");
        assert_eq!(
            engine.evaluate(Path::new("/p/a.py"), None, None).verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn default_action_block_applies_to_unmatched_files() {
        let engine = engine_from("default_action = \"block\"");
        let eval = engine.evaluate(Path::new("/p/README.md"), None, Some("haiku"));
        assert!(blocked(&eval.verdict));
    }

    #[test]
    fn disabled_config_allows_everything() {
        let engine = engine_from("enabled = false");
        assert_eq!(
            engine
                .evaluate(Path::new("/p/a.rs"), None, Some("opus"))
                .verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn disabled_rule_is_skipped() {
        let engine = engine_from("[[rule]]\nname = \"source-code\"\nenabled = false");
        assert_eq!(
            engine
                .evaluate(Path::new("/p/a.rs"), None, Some("opus"))
                .verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn model_substring_is_case_insensitive() {
        let engine = engine_from(
            "[[rule]]\nname = \"ts\"\nfiles = [\"*.ts\"]\nallowed_models = [\"HAIKU\"]",
        );
        assert_eq!(
            engine
                .evaluate(Path::new("/p/a.ts"), None, Some("claude-haiku-5-5"))
                .verdict,
            Verdict::Allow
        );
        assert!(blocked(
            &engine
                .evaluate(Path::new("/p/a.ts"), None, Some("claude-opus-5"))
                .verdict
        ));
    }

    #[test]
    fn model_glob_is_case_insensitive() {
        let engine = engine_from(
            "[[rule]]\nname = \"ts\"\nfiles = [\"*.ts\"]\nallowed_models = [\"*Sonnet*\"]",
        );
        assert_eq!(
            engine
                .evaluate(Path::new("/p/a.ts"), None, Some("claude-sonnet-5"))
                .verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn first_matching_rule_wins() {
        let engine = engine_from(
            "[[rule]]\nname = \"first\"\nfiles = [\"*.rs\"]\nallowed_models = [\"opus\"]\n\
             [[rule]]\nname = \"second\"\nfiles = [\"*.rs\"]\nallowed_models = [\"haiku\"]",
        );
        let eval = engine.evaluate(Path::new("/p/a.rs"), None, Some("opus"));
        assert_eq!(eval.rule.as_deref(), Some("first"));
        assert_eq!(eval.verdict, Verdict::Allow);
    }

    #[test]
    fn path_pattern_matches_relative_to_root() {
        let engine = engine_from(
            "[[rule]]\nname = \"gen\"\nfiles = [\"src/generated/**/*.rs\"]\nallowed_models = [\"never\"]",
        );
        let root = PathBuf::from("/proj");
        let inside = engine.evaluate(
            Path::new("/proj/src/generated/x/a.rs"),
            Some(&root),
            Some("haiku"),
        );
        assert_eq!(inside.rule.as_deref(), Some("gen"));
        let outside = engine.evaluate(
            Path::new("/proj/src/other/a.rs"),
            Some(&root),
            Some("haiku"),
        );
        assert_ne!(outside.rule.as_deref(), Some("gen"));
    }

    #[test]
    fn double_star_prefix_matches_absolute_paths() {
        let engine = engine_from(
            "[[rule]]\nname = \"any\"\nfiles = [\"**/*.rs\"]\nallowed_models = [\"haiku\"]",
        );
        let eval = engine.evaluate(Path::new("/some/where/a.rs"), None, Some("opus"));
        assert_eq!(eval.rule.as_deref(), Some("any"));
    }

    #[test]
    fn file_name_pattern_ignores_directory_names() {
        let engine = default_engine();
        // "*.rs" matches the file name, so a directory named like that does not count.
        let eval = engine.evaluate(Path::new("/p/notes.rs/readme.md"), None, Some("opus"));
        assert!(eval.rule.is_none());
    }

    #[test]
    fn custom_message_placeholders_render() {
        let engine = engine_from(
            "[[rule]]\nname = \"ts\"\nfiles = [\"*.ts\"]\nallowed_models = [\"haiku\"]\n\
             message = \"{rule}: {model} on {file}, allowed {allowed}\"",
        );
        match engine
            .evaluate(Path::new("/p/a.ts"), None, Some("opus"))
            .verdict
        {
            Verdict::Block { message } => assert_eq!(message, "ts: opus on /p/a.ts, allowed haiku"),
            Verdict::Allow => panic!("expected block"),
        }
    }

    #[test]
    fn invalid_glob_is_an_error() {
        let raw: RawConfig =
            toml::from_str("[[rule]]\nname = \"bad\"\nfiles = [\"[\"]\nallowed_models = [\"x\"]")
                .unwrap();
        let config = merge(&[Layer {
            source: "t".into(),
            raw,
        }])
        .unwrap();
        let err = Engine::new(&config).unwrap_err();
        assert!(err.to_string().contains("invalid file pattern"));
    }

    #[test]
    fn display_path_is_relative_inside_root() {
        assert_eq!(
            display_path(Path::new("/a/b/c.rs"), Some(Path::new("/a"))),
            "b/c.rs"
        );
        assert_eq!(
            display_path(Path::new("/x/c.rs"), Some(Path::new("/a"))),
            "/x/c.rs"
        );
    }
}
