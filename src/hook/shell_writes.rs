//! Finds the files that a Bash command writes to.
//!
//! The command is parsed with `brush-parser`, then every simple command is
//! walked, including commands inside compound commands, functions and
//! process substitutions. It detects:
//!
//! - output redirects: `>`, `>>`, `>|`, `<>`, `&>`, `&>>` (not `2>&1` or `>/dev/null`)
//! - `tee [-a] FILE...`
//! - `sed -i[SUFFIX] ... FILE...` (also `sed -i '' ...` on macOS)
//! - `cp` and `mv`, where the last operand is the target
//! - `dd of=FILE`
//!
//! Not detected: `python -c`, `perl -i`, `awk -i inplace`, `curl -o`, `wget -O`,
//! `git apply`, and commands run through `eval` or `$(...)`. Heredoc bodies are
//! not words, so text inside them does not count as a write.

use brush_parser::ast::{
    AndOr, AndOrList, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList,
    CompoundListItem, FunctionBody, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect, Pipeline,
    Program, RedirectList, SimpleCommand, Word,
};

/// Returns the file names that `command` writes to. Names are not resolved.
pub fn write_targets(command: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut parser = brush_parser::Parser::new(
        std::io::BufReader::new(command.as_bytes()),
        &brush_parser::ParserOptions::default(),
    );
    match parser.parse_program() {
        Ok(program) => walk_program(&program, &mut targets),
        Err(_) => fallback_targets(command, &mut targets),
    }

    targets.retain(|t| !t.is_empty() && !t.starts_with("/dev/"));
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    unique
}

/// Used when the parser rejects the input. Takes the word after each `>`.
/// Quoted text can match here, which is a false positive but errs on the safe side.
fn fallback_targets(command: &str, out: &mut Vec<String>) {
    let spaced = command
        .replace(">>", " > ")
        .replace(">|", " > ")
        .replace('>', " > ");
    let mut words = spaced.split_whitespace();
    while let Some(word) = words.next() {
        if word == ">" {
            if let Some(target) = words.next().filter(|t| !t.starts_with('&')) {
                out.push(target.to_string());
            }
        }
    }
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
        IoRedirect::File(_, kind, IoFileRedirectTarget::Filename(word)) if is_write(kind) => {
            out.push(unquote(&word.value));
        }
        IoRedirect::File(_, _, IoFileRedirectTarget::ProcessSubstitution(_, sub)) => {
            walk_list(&sub.list, out);
        }
        IoRedirect::OutputAndError(word, _) => out.push(unquote(&word.value)),
        _ => {}
    }
}

fn is_write(kind: &IoFileRedirectKind) -> bool {
    matches!(
        kind,
        IoFileRedirectKind::Write
            | IoFileRedirectKind::Append
            | IoFileRedirectKind::Clobber
            | IoFileRedirectKind::ReadAndWrite
    )
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

/// Decodes the body of a `$'...'` string. Unknown escapes keep the character.
fn ansi_c(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
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
}
