//! `modelgate explain`: shows which rule matches a file and why.

use std::path::Path;

use crate::config::{self, LoadOptions};
use crate::error::Result;
use crate::rules::{Engine, Verdict};

/// Explains the decision for `file` and `model`. Returns text for the terminal.
///
/// A relative `file` is taken from `cwd`. `model` of `None` means unknown.
pub fn explain(
    options: &LoadOptions,
    file: &Path,
    model: Option<&str>,
    cwd: &Path,
) -> Result<String> {
    let layers = config::load_layers(options)?;
    let merged = config::layers::merge(&layers)?;
    let engine = Engine::new(&merged)?;

    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        cwd.join(file)
    };
    let evaluation = engine.evaluate(&absolute, options.project_root.as_deref(), model);

    let mut out = String::new();
    out.push_str(&format!("file:          {}\n", evaluation.file));
    out.push_str(&format!(
        "model:         {}\n",
        evaluation.model.as_deref().unwrap_or("unknown")
    ));
    out.push_str(&format!(
        "rule:          {}\n",
        evaluation.rule.as_deref().unwrap_or("none")
    ));
    if let Some(pattern) = &evaluation.model_pattern {
        out.push_str(&format!("model match:   {pattern}\n"));
    }
    out.push_str(&format!("reason:        {}\n", evaluation.reason));
    match &evaluation.verdict {
        Verdict::Allow => out.push_str("decision:      allow\n"),
        Verdict::Block { message } => {
            out.push_str("decision:      block\n");
            out.push_str(&format!("message:       {message}\n"));
        }
    }
    out.push_str("layers:\n");
    for layer in &layers {
        out.push_str(&format!("  {}\n", layer.source));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_block_for_opus_on_rust() {
        let tmp = tempfile::tempdir().unwrap();
        let options = LoadOptions {
            config_dir: tmp.path().to_path_buf(),
            project_root: None,
        };
        let text = explain(
            &options,
            Path::new("src/lib.rs"),
            Some("claude-opus-5-5"),
            Path::new("/proj"),
        )
        .unwrap();
        assert!(text.contains("file:          /proj/src/lib.rs"), "{text}");
        assert!(text.contains("rule:          source-code"), "{text}");
        assert!(text.contains("decision:      block"), "{text}");
        assert!(
            text.contains("Send this edit to a Haiku subagent."),
            "{text}"
        );
    }

    #[test]
    fn explains_allow_for_haiku_on_rust() {
        let tmp = tempfile::tempdir().unwrap();
        let options = LoadOptions {
            config_dir: tmp.path().to_path_buf(),
            project_root: None,
        };
        let text = explain(
            &options,
            Path::new("/proj/a.rs"),
            Some("claude-haiku-5-5"),
            Path::new("/"),
        )
        .unwrap();
        assert!(text.contains("decision:      allow"), "{text}");
        assert!(text.contains("model match:   *haiku*"), "{text}");
    }
}
