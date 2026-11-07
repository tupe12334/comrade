//! Message templates with `{placeholder}` values.

/// Placeholder names a message template may use.
pub const PLACEHOLDERS: &[&str] = &["file", "model", "allowed", "rule"];

/// Replaces every known `{name}` in `template` with its value.
///
/// Unknown `{...}` text is kept as written.
pub fn render(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match values.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Returns every `{name}` in `template` whose name is not a known placeholder.
pub fn unknown_placeholders(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        let name = &after[..close];
        let is_name =
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if is_name && !PLACEHOLDERS.contains(&name) {
            found.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_known_placeholders() {
        let out = render(
            "{model} may not edit {file}",
            &[("model", "opus"), ("file", "a.rs")],
        );
        assert_eq!(out, "opus may not edit a.rs");
    }

    #[test]
    fn keeps_unknown_placeholders() {
        assert_eq!(render("{nope} {file}", &[("file", "a.rs")]), "{nope} a.rs");
    }

    #[test]
    fn keeps_unclosed_brace() {
        assert_eq!(render("bad { text", &[]), "bad { text");
    }

    #[test]
    fn finds_unknown_names_only() {
        assert_eq!(
            unknown_placeholders("{file} {typo} {model} {x_y}"),
            vec!["typo", "x_y"]
        );
    }

    #[test]
    fn ignores_non_name_braces() {
        assert!(unknown_placeholders("json: {\"a\": 1}").is_empty());
    }
}
