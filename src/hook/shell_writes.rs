//! Finds the files that a simple Bash command writes to.
//!
//! This is a best-effort scan, not a shell parser. It detects:
//!
//! - output redirects: `>`, `>>`, `>|`, `&>`, `&>>` (not `2>&1` or `>/dev/null`)
//! - `tee [-a] FILE...`
//! - `sed -i[SUFFIX] ... FILE...` (also `sed -i '' ...` on macOS)
//! - `cp` and `mv`, where the last operand is the target
//! - `dd of=FILE`
//!
//! Not detected: `python -c`, `perl -i`, `awk -i inplace`, `curl -o`, `wget -O`,
//! `git apply`, and commands run through `eval` or `$(...)`. Heredoc bodies are
//! skipped, so text inside them does not count as a write.

/// Returns the file names that `command` writes to. Names are not resolved.
pub fn write_targets(command: &str) -> Vec<String> {
    let text = strip_heredoc_bodies(command);
    let tokens = tokenize(&text);

    let mut targets = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut pending = Pending::None;

    for token in tokens {
        match token {
            Token::Separator => {
                analyze(&args, &mut targets);
                args.clear();
                pending = Pending::None;
            }
            Token::Word(word) => match pending {
                Pending::Target => {
                    targets.push(word);
                    pending = Pending::None;
                }
                Pending::Skip => pending = Pending::None,
                Pending::None => args.push(word),
            },
            Token::Redirect => pending = Pending::Target,
            Token::Input | Token::Dup => pending = Pending::Skip,
        }
    }
    analyze(&args, &mut targets);

    targets.retain(|t| !t.is_empty() && !t.starts_with("/dev/"));
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    /// The next word is a write target.
    Target,
    /// The next word is not a file (for example an fd number or an input).
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    /// An output redirect such as `>` or `>>`.
    Redirect,
    /// An input redirect such as `<`.
    Input,
    /// A file descriptor duplication such as `>&`.
    Dup,
    /// A command separator such as `;`, `&&`, `|` or a newline.
    Separator,
}

/// Removes the body lines of heredocs, keeping the lines that start them.
fn strip_heredoc_bodies(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut delimiter: Option<String> = None;
    for line in command.lines() {
        if let Some(end) = &delimiter {
            if line.trim() == end {
                delimiter = None;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        delimiter = heredoc_delimiter(line);
    }
    out
}

fn heredoc_delimiter(line: &str) -> Option<String> {
    let index = line.find("<<")?;
    let rest = &line[index + 2..];
    if rest.starts_with('<') {
        return None; // here-string, not a heredoc
    }
    let rest = rest.trim_start_matches('-').trim_start();
    let word: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, ';' | '|' | '&' | ')'))
        .collect();
    let word = word.trim_matches(|c| c == '\'' || c == '"');
    if word.is_empty() {
        None
    } else {
        Some(word.to_string())
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut word: Option<String> = None;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                let buffer = word.get_or_insert_with(String::new);
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    buffer.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                let buffer = word.get_or_insert_with(String::new);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\'
                        && i + 1 < chars.len()
                        && matches!(chars[i + 1], '"' | '\\' | '$' | '`')
                    {
                        buffer.push(chars[i + 1]);
                        i += 2;
                    } else {
                        buffer.push(chars[i]);
                        i += 1;
                    }
                }
                i += 1;
            }
            '\\' => {
                if i + 1 < chars.len() {
                    word.get_or_insert_with(String::new).push(chars[i + 1]);
                }
                i += 2;
            }
            ' ' | '\t' | '\r' => {
                flush(&mut word, &mut tokens);
                i += 1;
            }
            '\n' | ';' | '|' | '(' | ')' => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Separator);
                i += 1;
            }
            '&' if chars.get(i + 1) == Some(&'>') => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Redirect);
                i += 2;
                if chars.get(i) == Some(&'>') {
                    i += 1;
                }
            }
            '&' => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Separator);
                i += 1;
            }
            '>' => {
                // A digit word right before `>` is a file descriptor, such as `2>`.
                let fd_prefix = word
                    .as_deref()
                    .is_some_and(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()));
                if fd_prefix {
                    word = None;
                } else {
                    flush(&mut word, &mut tokens);
                }
                i += 1;
                if chars.get(i) == Some(&'>') || chars.get(i) == Some(&'|') {
                    i += 1;
                }
                if chars.get(i) == Some(&'&') {
                    i += 1;
                    tokens.push(Token::Dup);
                } else {
                    tokens.push(Token::Redirect);
                }
            }
            '<' => {
                flush(&mut word, &mut tokens);
                tokens.push(Token::Input);
                while chars.get(i) == Some(&'<') {
                    i += 1;
                }
                if matches!(chars.get(i), Some('&') | Some('>')) {
                    i += 1;
                }
            }
            _ => {
                word.get_or_insert_with(String::new).push(c);
                i += 1;
            }
        }
    }
    flush(&mut word, &mut tokens);
    tokens
}

fn flush(word: &mut Option<String>, tokens: &mut Vec<Token>) {
    if let Some(w) = word.take() {
        tokens.push(Token::Word(w));
    }
}

/// Looks at one simple command and adds the files it writes to.
fn analyze(args: &[String], out: &mut Vec<String>) {
    let Some(program) = args.first() else {
        return;
    };
    let rest = &args[1..];
    match basename(program) {
        "tee" => out.extend(rest.iter().filter(|a| !a.starts_with('-')).cloned()),
        "sed" if sed_in_place(rest) => out.extend(sed_files(rest)),
        "cp" | "mv" | "install" => {
            let operands: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
            if operands.len() >= 2 {
                out.push(operands[operands.len() - 1].clone());
            }
        }
        "dd" => out.extend(
            rest.iter()
                .filter_map(|a| a.strip_prefix("of="))
                .map(str::to_string),
        ),
        _ => {}
    }
}

fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

fn sed_in_place(args: &[String]) -> bool {
    args.iter().any(|a| {
        a == "--in-place"
            || a.starts_with("--in-place=")
            || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('i'))
    })
}

/// Returns the files after the sed script. Handles `-e`, `-f` and `sed -i ''`.
fn sed_files(args: &[String]) -> Vec<String> {
    let mut script_given = false;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-e" || arg == "-f" {
            script_given = true;
            i += 2;
            continue;
        }
        if arg == "-i" && args.get(i + 1).is_some_and(String::is_empty) {
            i += 2;
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            i += 1;
            continue;
        }
        positional.push(arg.clone());
        i += 1;
    }
    if script_given {
        positional
    } else {
        positional.into_iter().skip(1).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::write_targets;

    fn targets(cmd: &str) -> Vec<String> {
        write_targets(cmd)
    }

    #[test]
    fn detects_output_redirect() {
        assert_eq!(targets("echo hi > src/main.rs"), vec!["src/main.rs"]);
        assert_eq!(targets("echo hi >> a.py"), vec!["a.py"]);
        assert_eq!(targets("echo hi >| b.go"), vec!["b.go"]);
        assert_eq!(targets("make && echo x > out.sh"), vec!["out.sh"]);
    }

    #[test]
    fn ignores_fd_dups_and_null() {
        assert!(targets("cargo build 2>&1").is_empty());
        assert!(targets("echo x > /dev/null").is_empty());
        assert!(targets("cmd 2>/dev/null").is_empty());
    }

    #[test]
    fn detects_combined_stdout_stderr_redirect() {
        assert_eq!(targets("cmd &> all.log"), vec!["all.log"]);
    }

    #[test]
    fn heredoc_body_is_not_a_write() {
        let cmd = "cat > lib.rs <<'EOF'\nlet x = 1; // > fake.rs\nEOF\n";
        assert_eq!(targets(cmd), vec!["lib.rs"]);
    }

    #[test]
    fn quoted_redirect_is_text() {
        assert!(targets("echo \"a > b.rs\"").is_empty());
        assert!(targets("echo 'x >> y.ts'").is_empty());
    }

    #[test]
    fn input_redirect_is_not_a_write() {
        assert!(targets("wc -l < notes.rs").is_empty());
    }

    #[test]
    fn detects_tee() {
        assert_eq!(
            targets("cargo build 2>&1 | tee build.log"),
            vec!["build.log"]
        );
        assert_eq!(targets("echo x | tee -a a.ts b.ts"), vec!["a.ts", "b.ts"]);
    }

    #[test]
    fn detects_sed_in_place() {
        assert_eq!(targets("sed -i 's/a/b/' x.ts"), vec!["x.ts"]);
        assert_eq!(targets("sed -i.bak 's/a/b/' x.ts"), vec!["x.ts"]);
        assert_eq!(
            targets("sed -i '' -e 's/a/b/' y.go z.go"),
            vec!["y.go", "z.go"]
        );
        assert_eq!(targets("sed -E -i 's/a/b/' w.c"), vec!["w.c"]);
    }

    #[test]
    fn sed_without_in_place_writes_nothing() {
        assert!(targets("sed 's/a/b/' x.ts").is_empty());
        assert!(targets("sed -n 's/a/b/p' x.ts").is_empty());
    }

    #[test]
    fn detects_cp_mv_and_dd() {
        assert_eq!(targets("cp a.rs b.rs"), vec!["b.rs"]);
        assert_eq!(targets("mv -f tmp.py final.py"), vec!["final.py"]);
        assert_eq!(targets("dd if=x of=y.c"), vec!["y.c"]);
    }

    #[test]
    fn plain_commands_write_nothing() {
        assert!(targets("ls -la").is_empty());
        assert!(targets("cargo test").is_empty());
    }
}
