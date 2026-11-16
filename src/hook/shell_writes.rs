//! Finds the files that a Bash command writes to.
//!
//! The command is parsed with [`brush_parser`], a bash-compatible parser, so
//! quoting, comments, heredocs, subshells, `{ ...; }` groups, `if`/`for`/`while`
//! bodies and `>(...)` process substitutions follow shell rules. It detects:
//!
//! - output redirects: `>`, `>>`, `>|`, `<>`, `&>`, `&>>` (not `2>&1` or `>/dev/null`)
//! - `tee [-a] FILE...`
//! - `sed -i[SUFFIX] ... FILE...` (also `sed -i '' ...` on macOS)
//! - `cp` and `mv`, where the last operand is the target
//! - `dd of=FILE`
//!
//! Not detected: `python -c`, `perl -i`, `awk -i inplace`, `curl -o`, `wget -O`,
//! `git apply`, and commands run through `eval`, `$(...)` or backticks.
//! A command that does not parse as bash yields no targets: bash would refuse
//! to run it too.

use brush_parser::ast::{
    AndOr, AndOrList, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList,
    IoFileRedirectKind, IoFileRedirectTarget, IoRedirect, RedirectList, SimpleCommand,
};
use brush_parser::word::{self, WordPiece, WordPieceWithSource};
use brush_parser::{Parser, ParserOptions};

/// Returns the file names that `command` writes to. Names are not resolved.
pub fn write_targets(command: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut reader = std::io::BufReader::new(command.as_bytes());
    let mut parser = Parser::new(&mut reader, &ParserOptions::default());
    if let Ok(program) = parser.parse_program() {
        for list in &program.complete_commands {
            visit_list(list, &mut targets);
        }
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

fn visit_list(list: &CompoundList, out: &mut Vec<String>) {
    for item in &list.0 {
        visit_and_or(&item.0, out);
    }
}

fn visit_and_or(list: &AndOrList, out: &mut Vec<String>) {
    let rest = list.additional.iter().map(|next| match next {
        AndOr::And(p) | AndOr::Or(p) => p,
    });
    for pipeline in std::iter::once(&list.first).chain(rest) {
        for command in &pipeline.seq {
            visit_command(command, out);
        }
    }
}

fn visit_command(command: &Command, out: &mut Vec<String>) {
    match command {
        Command::Simple(simple) => visit_simple(simple, out),
        Command::Compound(compound, redirects) => {
            visit_compound(compound, out);
            visit_redirects(redirects.as_ref(), out);
        }
        Command::Function(function) => {
            visit_compound(&function.body.0, out);
            visit_redirects(function.body.1.as_ref(), out);
        }
        Command::ExtendedTest(_, redirects) => visit_redirects(redirects.as_ref(), out),
    }
}

fn visit_compound(compound: &CompoundCommand, out: &mut Vec<String>) {
    match compound {
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::ArithmeticForClause(c) => visit_list(&c.body.list, out),
        CompoundCommand::BraceGroup(c) => visit_list(&c.list, out),
        CompoundCommand::Subshell(c) => visit_list(&c.list, out),
        CompoundCommand::ForClause(c) => visit_list(&c.body.list, out),
        CompoundCommand::CaseClause(c) => {
            for case in &c.cases {
                if let Some(list) = &case.cmd {
                    visit_list(list, out);
                }
            }
        }
        CompoundCommand::IfClause(c) => {
            visit_list(&c.condition, out);
            visit_list(&c.then, out);
            for clause in c.elses.iter().flatten() {
                if let Some(condition) = &clause.condition {
                    visit_list(condition, out);
                }
                visit_list(&clause.body, out);
            }
        }
        CompoundCommand::WhileClause(c) | CompoundCommand::UntilClause(c) => {
            visit_list(&c.0, out);
            visit_list(&c.1.list, out);
        }
        CompoundCommand::Coprocess(c) => visit_command(&c.body, out),
    }
}

/// Looks at one simple command: its redirects, then its program and arguments.
fn visit_simple(command: &SimpleCommand, out: &mut Vec<String>) {
    let mut args: Vec<String> = command
        .word_or_name
        .iter()
        .map(|w| unquote(&w.value))
        .collect();
    // A prefix `NAME=value` sets the environment. After the program name it is
    // a plain argument, such as `dd of=FILE`.
    let prefix = command.prefix.iter().flat_map(|p| &p.0).map(|i| (i, false));
    let suffix = command.suffix.iter().flat_map(|s| &s.0).map(|i| (i, true));
    for (item, is_argument) in prefix.chain(suffix) {
        match item {
            CommandPrefixOrSuffixItem::IoRedirect(redirect) => visit_redirect(redirect, out),
            CommandPrefixOrSuffixItem::Word(w) => args.push(unquote(&w.value)),
            CommandPrefixOrSuffixItem::AssignmentWord(_, w) if is_argument => {
                args.push(unquote(&w.value));
            }
            CommandPrefixOrSuffixItem::AssignmentWord(..) => {}
            CommandPrefixOrSuffixItem::ProcessSubstitution(_, sub) => visit_list(&sub.list, out),
        }
    }
    analyze(&args, out);
}

fn visit_redirects(redirects: Option<&RedirectList>, out: &mut Vec<String>) {
    for redirect in redirects.iter().flat_map(|r| &r.0) {
        visit_redirect(redirect, out);
    }
}

fn visit_redirect(redirect: &IoRedirect, out: &mut Vec<String>) {
    match redirect {
        IoRedirect::File(
            _,
            IoFileRedirectKind::Write
            | IoFileRedirectKind::Append
            | IoFileRedirectKind::Clobber
            | IoFileRedirectKind::ReadAndWrite,
            IoFileRedirectTarget::Filename(w),
        ) => out.push(unquote(&w.value)),
        IoRedirect::File(_, _, IoFileRedirectTarget::ProcessSubstitution(_, sub)) => {
            visit_list(&sub.list, out);
        }
        IoRedirect::OutputAndError(w, _) => out.push(unquote(&w.value)),
        _ => {}
    }
}

/// Removes shell quoting from a word. Expansions such as `$HOME` or `$(...)`
/// stay as written, because they are not run.
fn unquote(raw: &str) -> String {
    match word::parse(raw, &ParserOptions::default()) {
        Ok(pieces) => pieces.iter().map(|p| piece_text(raw, p)).collect(),
        Err(_) => raw.to_string(),
    }
}

fn piece_text(raw: &str, piece: &WordPieceWithSource) -> String {
    match &piece.piece {
        WordPiece::Text(s) | WordPiece::SingleQuotedText(s) | WordPiece::AnsiCQuotedText(s) => {
            s.clone()
        }
        WordPiece::EscapeSequence(s) => s.strip_prefix('\\').unwrap_or(s).to_string(),
        WordPiece::DoubleQuotedSequence(inner) | WordPiece::GettextDoubleQuotedSequence(inner) => {
            inner.iter().map(|p| piece_text(raw, p)).collect()
        }
        _ => raw
            .get(piece.start_index..piece.end_index)
            .unwrap_or_default()
            .to_string(),
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

    #[test]
    fn detects_writes_inside_compound_commands() {
        assert_eq!(targets("if true; then cp a.rs b.rs; fi"), vec!["b.rs"]);
        assert_eq!(targets("for f in a b; do cp x $f.rs; done"), vec!["$f.rs"]);
        assert_eq!(
            targets("while true; do echo x | tee w.rs; done"),
            vec!["w.rs"]
        );
        assert_eq!(targets("{ echo a; echo b; } > out.rs"), vec!["out.rs"]);
        assert_eq!(targets("(cd src && sed -i 's/a/b/' m.rs)"), vec!["m.rs"]);
    }

    #[test]
    fn comment_is_not_a_write() {
        assert!(targets("ls -la # > c.rs").is_empty());
        assert_eq!(targets("echo hi > a.rs # > b.rs"), vec!["a.rs"]);
    }

    #[test]
    fn ansi_c_quoted_target_is_unquoted() {
        assert_eq!(targets("echo hi > $'src/a.rs'"), vec!["src/a.rs"]);
    }

    #[test]
    fn detects_process_substitution_writes() {
        assert_eq!(targets("make > >(tee build.rs)"), vec!["build.rs"]);
    }

    #[test]
    fn unparseable_command_writes_nothing() {
        assert!(targets("echo 'unterminated > x.rs").is_empty());
    }
}
