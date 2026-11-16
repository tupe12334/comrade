//! Finds the files that a Bash command writes to.
//!
//! The command is parsed with `brush-parser`, then every simple command is
//! walked, including commands inside compound commands, functions and
//! process substitutions. It detects:
//!
//! - output redirects: `>`, `>>`, `>|`, `<>`, `&>`, `&>>` (not `2>&1` or `>/dev/null`)
//! - `tee [-a] FILE...`
//! - `sed -i[SUFFIX] ... FILE...` (also `sed -i '' ...` on macOS)
//! - `cp`, `mv` and `install`, where the last operand is the target
//! - `dd of=FILE`
//! - `$(...)` and backtick substitutions, analyzed as commands of their own
//!
//! Not detected: `python -c`, `perl -i`, `awk -i inplace`, `curl -o`, `wget -O`,
//! `git apply`, and commands run through `eval`. Heredoc bodies are not words,
//! so text inside them does not count as a write. Input the parser cannot
//! handle, or input that is too large or too nested, uses a conservative scan
//! (see `shell_fallback`).

use brush_parser::ast::{
    AndOr, AndOrList, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList,
    CompoundListItem, FunctionBody, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect, Pipeline,
    Program, RedirectList, SimpleCommand, Word,
};

use super::shell_fallback::fallback_targets;
use super::shell_words::{ansi_c, is_fd_word};

/// Longest command, in bytes, that the parser runs on.
const MAX_PARSED_LEN: usize = 64 * 1024;
/// Most `(`, `{`, backticks and compound keywords the parser runs on. Bounds recursion depth.
const MAX_PARSED_OPENERS: usize = 64;
/// Deepest chain of substitutions followed. Beyond this, the fallback scan takes over.
const MAX_SUBSTITUTION_DEPTH: usize = 8;
const COMPOUND_KEYWORDS: [&str; 6] = ["if", "while", "until", "for", "case", "select"];

/// Returns the file names that `command` writes to. Names are not resolved.
pub fn write_targets(command: &str) -> Vec<String> {
    let mut targets = Vec::new();
    collect_targets(command, 0, &mut targets);

    targets.retain(|t| !t.is_empty() && !t.starts_with("/dev/"));
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}

fn collect_targets(command: &str, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_SUBSTITUTION_DEPTH || too_large_to_parse(command) {
        return fallback_targets(command, out);
    }
    let mut parser = brush_parser::Parser::new(
        std::io::BufReader::new(command.as_bytes()),
        &brush_parser::ParserOptions::default(),
    );
    match parser.parse_program() {
        Ok(program) => {
            walk_program(&program, out);
            for body in substitution_bodies(command) {
                collect_targets(body, depth + 1, out);
            }
        }
        Err(_) => fallback_targets(command, out),
    }
}

/// True when the input is long or has many openers, so the parser must not see it.
fn too_large_to_parse(command: &str) -> bool {
    let symbols = command
        .chars()
        .filter(|c| matches!(c, '(' | '{' | '`'))
        .count();
    let keywords = command
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| COMPOUND_KEYWORDS.contains(w))
        .count();
    command.len() > MAX_PARSED_LEN || symbols + keywords > MAX_PARSED_OPENERS
}

/// Returns the text inside each `$(...)` and backtick substitution that is not in single quotes.
fn substitution_bodies(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut bodies = Vec::new();
    let (mut single, mut double) = (false, false);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        match c {
            b'\\' if !single => i += 1,
            b'\'' if !double => single = !single,
            b'"' if !single => double = !double,
            b'$' if !single && bytes.get(i) == Some(&b'(') => {
                let start = i + 1;
                let end = matching_paren(bytes, start);
                bodies.push(&text[start..end]);
                i = end + 1;
            }
            b'`' if !single => {
                let end = bytes[i..]
                    .iter()
                    .position(|&b| b == b'`')
                    .map_or(bytes.len(), |p| i + p);
                bodies.push(&text[i..end]);
                i = end + 1;
            }
            _ => {}
        }
    }
    bodies
}

/// Returns the index of the `)` that closes a `(` just before `start`, or the end of `bytes`.
fn matching_paren(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' if depth == 0 => return start + offset,
            b')' => depth -= 1,
            _ => {}
        }
    }
    bytes.len()
}

fn walk_program(program: &Program, out: &mut Vec<String>) {
    for complete in &program.complete_commands {
        walk_list(complete, out);
    }
}

fn walk_list(list: &CompoundList, out: &mut Vec<String>) {
    for CompoundListItem(and_or, _) in &list.0 {
        walk_and_or(and_or, out);
    }
}

fn walk_and_or(list: &AndOrList, out: &mut Vec<String>) {
    walk_pipeline(&list.first, out);
    for item in &list.additional {
        match item {
            AndOr::And(pipeline) | AndOr::Or(pipeline) => walk_pipeline(pipeline, out),
        }
    }
}

fn walk_pipeline(pipeline: &Pipeline, out: &mut Vec<String>) {
    for command in &pipeline.seq {
        walk_command(command, out);
    }
}

fn walk_command(command: &Command, out: &mut Vec<String>) {
    match command {
        Command::Simple(simple) => walk_simple(simple, out),
        Command::Compound(compound, redirects) => {
            walk_compound(compound, out);
            walk_redirects(redirects.as_ref(), out);
        }
        Command::Function(definition) => {
            let FunctionBody(compound, redirects) = &definition.body;
            walk_compound(compound, out);
            walk_redirects(redirects.as_ref(), out);
        }
        Command::ExtendedTest(_, redirects) => walk_redirects(redirects.as_ref(), out),
    }
}

fn walk_compound(compound: &CompoundCommand, out: &mut Vec<String>) {
    match compound {
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::ArithmeticForClause(c) => walk_list(&c.body.list, out),
        CompoundCommand::BraceGroup(b) => walk_list(&b.list, out),
        CompoundCommand::Subshell(s) => walk_list(&s.list, out),
        CompoundCommand::ForClause(f) => walk_list(&f.body.list, out),
        CompoundCommand::CaseClause(c) => {
            for case in &c.cases {
                if let Some(list) = &case.cmd {
                    walk_list(list, out);
                }
            }
        }
        CompoundCommand::IfClause(c) => {
            walk_list(&c.condition, out);
            walk_list(&c.then, out);
            for clause in c.elses.iter().flatten() {
                if let Some(condition) = &clause.condition {
                    walk_list(condition, out);
                }
                walk_list(&clause.body, out);
            }
        }
        CompoundCommand::WhileClause(w) | CompoundCommand::UntilClause(w) => {
            walk_list(&w.0, out);
            walk_list(&w.1.list, out);
        }
        CompoundCommand::Coprocess(c) => walk_command(&c.body, out),
    }
}

fn walk_simple(simple: &SimpleCommand, out: &mut Vec<String>) {
    // Prefix assignments such as `FOO=1 tee x` are not argv words.
    if let Some(prefix) = &simple.prefix {
        for item in &prefix.0 {
            walk_item(item, out);
        }
    }
    let mut argv = Vec::new();
    if let Some(word) = &simple.word_or_name {
        argv.push(unquote(&word.value));
    }
    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            if let Some(word) = walk_item(item, out) {
                argv.push(unquote(&word.value));
            }
        }
    }
    analyze(&argv, out);
}

/// Handles redirects and process substitutions. Returns the word for argv items.
fn walk_item<'a>(item: &'a CommandPrefixOrSuffixItem, out: &mut Vec<String>) -> Option<&'a Word> {
    match item {
        CommandPrefixOrSuffixItem::IoRedirect(redirect) => {
            walk_redirect(redirect, out);
            None
        }
        CommandPrefixOrSuffixItem::ProcessSubstitution(_, sub) => {
            walk_list(&sub.list, out);
            None
        }
        CommandPrefixOrSuffixItem::Word(word)
        | CommandPrefixOrSuffixItem::AssignmentWord(_, word) => Some(word),
    }
}

fn walk_redirects(redirects: Option<&RedirectList>, out: &mut Vec<String>) {
    if let Some(RedirectList(items)) = redirects {
        for redirect in items {
            walk_redirect(redirect, out);
        }
    }
}

fn walk_redirect(redirect: &IoRedirect, out: &mut Vec<String>) {
    match redirect {
        IoRedirect::File(
            _,
            kind,
            IoFileRedirectTarget::Filename(word) | IoFileRedirectTarget::Duplicate(word),
        ) => {
            let target = unquote(&word.value);
            if is_write(kind, &target) {
                out.push(target);
            }
        }
        IoRedirect::File(_, _, IoFileRedirectTarget::ProcessSubstitution(_, sub)) => {
            walk_list(&sub.list, out);
        }
        IoRedirect::OutputAndError(word, _) => out.push(unquote(&word.value)),
        _ => {}
    }
}

/// True when a redirect of this kind to `target` opens a file for writing.
/// `>&WORD` writes to WORD unless WORD names a descriptor.
fn is_write(kind: &IoFileRedirectKind, target: &str) -> bool {
    match kind {
        IoFileRedirectKind::Write
        | IoFileRedirectKind::Append
        | IoFileRedirectKind::Clobber
        | IoFileRedirectKind::ReadAndWrite => true,
        IoFileRedirectKind::DuplicateOutput => !is_fd_word(target),
        IoFileRedirectKind::Read | IoFileRedirectKind::DuplicateInput => false,
    }
}

/// Removes shell quoting from one word. Falls back to the raw text.
fn unquote(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("$'") {
        // ANSI-C quoting: find the closing quote, skipping escaped characters.
        let mut escaped = false;
        for (i, c) in rest.char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '\'' {
                return ansi_c(&rest[..i]) + &unquote(&rest[i + 1..]);
            }
        }
        return raw.to_string();
    }
    match shlex::split(raw) {
        Some(mut parts) if parts.len() == 1 => parts.remove(0),
        _ => raw.to_string(),
    }
}

/// Looks at one simple command and adds the files it writes to.
pub(super) fn analyze(args: &[String], out: &mut Vec<String>) {
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

    #[test]
    fn quoted_angle_bracket_does_not_hide_later_write() {
        assert_eq!(
            targets("echo \"a<<X\"\necho hi > src/main.rs"),
            vec!["src/main.rs"]
        );
    }

    #[test]
    fn ansi_c_quoted_target_is_unquoted() {
        assert_eq!(targets("echo $'it\\'s' > src/main.rs"), vec!["src/main.rs"]);
    }

    #[test]
    fn multiple_heredocs_on_one_line() {
        assert_eq!(
            targets("cat <<A <<B\na\nA\nb > nope.rs\nB\necho > real.rs"),
            vec!["real.rs"]
        );
    }

    #[test]
    fn read_write_fd_open_is_a_write() {
        assert_eq!(targets("exec 3<>src/main.rs"), vec!["src/main.rs"]);
    }

    #[test]
    fn writes_inside_compound_commands_are_found() {
        assert_eq!(targets("if true; then echo a > x.rs; fi"), vec!["x.rs"]);
        assert_eq!(targets("for i in 1; do echo $i; done > y.rs"), vec!["y.rs"]);
        assert_eq!(targets("f() { echo a > z.rs; }; f"), vec!["z.rs"]);
    }

    #[test]
    fn parse_error_falls_back_to_whitespace_scan() {
        assert_eq!(targets("echo a > b.rs ) )"), vec!["b.rs"]);
    }

    #[test]
    fn parse_error_keeps_cp_and_tee_checks() {
        assert_eq!(targets("cp /tmp/x src/main.rs\n)"), vec!["src/main.rs"]);
        assert_eq!(targets("echo hi | tee src/main.rs\n)"), vec!["src/main.rs"]);
    }

    #[test]
    fn parse_error_keeps_sed_mv_install_dd_checks() {
        assert_eq!(targets("sed -i 's/a/b/' src/a.rs\n)"), vec!["src/a.rs"]);
        assert_eq!(targets("mv tmp.rs src/b.rs\n)"), vec!["src/b.rs"]);
        assert_eq!(targets("install x src/c.rs\n)"), vec!["src/c.rs"]);
        assert_eq!(targets("dd if=x of=src/d.rs\n)"), vec!["src/d.rs"]);
    }

    #[test]
    fn parse_error_fallback_removes_quotes() {
        assert_eq!(targets("echo x > 'src/main.rs'\n)"), vec!["src/main.rs"]);
        assert_eq!(targets("echo x > \"src/main.rs\"\n)"), vec!["src/main.rs"]);
    }

    #[test]
    fn parse_error_fallback_still_reads_fd_and_read_only_commands() {
        assert!(targets("cat src/main.rs\n)").is_empty());
        assert!(targets("cargo build 2>&1\n)").is_empty());
    }

    #[test]
    fn detects_writes_inside_dollar_paren_substitution() {
        assert_eq!(
            targets("echo $(cp /tmp/x src/main.rs)"),
            vec!["src/main.rs"]
        );
        assert_eq!(
            targets("echo \"$(cp /tmp/x src/main.rs)\""),
            vec!["src/main.rs"]
        );
    }

    #[test]
    fn detects_writes_inside_assignment_substitution() {
        assert_eq!(targets("x=$(tee src/main.rs <<<code)"), vec!["src/main.rs"]);
    }

    #[test]
    fn detects_writes_inside_backtick_substitution() {
        assert_eq!(targets("echo `cp /tmp/x src/main.rs`"), vec!["src/main.rs"]);
    }

    #[test]
    fn single_quoted_substitution_is_text() {
        assert!(targets("echo '$(cp /tmp/x src/main.rs)'").is_empty());
        assert!(targets("echo \"$(cat src/main.rs)\"").is_empty());
    }

    #[test]
    fn deeply_nested_parens_still_find_the_write() {
        let depth = 500;
        let cmd = format!(
            "{}echo x > src/main.rs{}",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        assert_eq!(targets(&cmd), vec!["src/main.rs"]);
    }

    #[test]
    fn fifty_thousand_nested_parens_do_not_overflow() {
        let depth = 50_000;
        let cmd = format!(
            "{}echo x > src/main.rs{}",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        assert_eq!(targets(&cmd), vec!["src/main.rs"]);
    }

    #[test]
    fn duplicate_output_to_a_file_is_a_write() {
        assert_eq!(targets("echo x >&src/main.rs"), vec!["src/main.rs"]);
        assert_eq!(targets("echo x >& src/main.rs"), vec!["src/main.rs"]);
        assert!(targets("echo x >&2").is_empty());
        assert!(targets("echo x 3>&1 1>&2 2>&3-").is_empty());
        assert!(targets("exec 3>&-").is_empty());
    }

    #[test]
    fn parse_error_fallback_finds_duplicate_output_to_a_file() {
        assert_eq!(targets("echo x >&src/main.rs\n)"), vec!["src/main.rs"]);
        assert_eq!(targets("echo x >& src/main.rs\n)"), vec!["src/main.rs"]);
        assert!(targets("cargo build >&2\n)").is_empty());
    }

    #[test]
    fn ansi_c_escapes_are_decoded_like_bash() {
        assert_eq!(targets("echo x > $'\\x73rc/main.rs'"), vec!["src/main.rs"]);
        assert_eq!(targets("echo x > $'\\163rc/main.rs'"), vec!["src/main.rs"]);
        assert_eq!(
            targets("echo x > $'\\u0073rc/main.rs'"),
            vec!["src/main.rs"]
        );
        assert_eq!(
            targets("echo x > $'src/main.rs\\x00.txt'"),
            vec!["src/main.rs"]
        );
    }

    #[test]
    fn parse_error_fallback_decodes_ansi_c_escapes() {
        assert_eq!(
            targets("echo x > $'\\x73rc/main.rs'\n)"),
            vec!["src/main.rs"]
        );
    }
}
