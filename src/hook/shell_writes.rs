//! Finds the files that a Bash command writes to.
//!
//! The command is parsed with `brush-parser`, then every simple command is
//! walked, including commands inside compound commands, functions and
//! process substitutions. It detects:
//!
//! - output redirects: `>`, `>>`, `>|`, `<>`, `&>`, `&>>` and `>&FILE` (not `2>&1` or `>/dev/null`)
//! - `tee [-a] FILE...`
//! - `sed -i[SUFFIX] ... FILE...` (also `sed -i '' ...` on macOS)
//! - `cp`, `mv` and `install`, where the last operand is the target
//! - `dd of=FILE`
//! - `$(...)` and backtick substitutions in any word, in `${...}` and `$((...))`,
//!   and in heredoc bodies that expand, analyzed as commands of their own
//!
//! Each word is split into pieces with `brush_parser::word`, so quotes and `$'...'`
//! are removed as bash removes them.
//!
//! Not detected:
//!
//! - `python -c`, `perl -i`, `awk -i inplace`, `curl -o`, `wget -O`, `git apply`
//! - commands run through `eval`
//! - `bash -c` and `sh -c`
//! - `env`, `sudo`, `command` and `nohup` wrappers, for example `env cp a src/main.rs`, `sudo tee`, `command cp`, `nohup tee`
//! - `find -exec`
//! - `touch`, `truncate`, `ln`, `rsync`, `patch`, `sponge` and `tar`
//! - `cp` and `mv` with `-t` or `--target-directory`
//! - `sed` with `--expression=` or `--file=`
//! - `cd` in the same command
//! - the `/dev/..` path form
//!
//! A substitution or variable inside a target name ($(...), $VAR, ~, globs) is not resolved.
//!
//! Text in a heredoc body is not a command. If any part of the input,
//! a substitution body included, does not parse, or is too large or too nested,
//! the whole input uses a conservative scan (see `shell_fallback`). An arithmetic
//! command `((...))` also adds that scan, because the parser reads `( (` as `((`.

use brush_parser::ast::{
    AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList, CompoundListItem,
    ExtendedTestExpr, FunctionBody, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect, Pipeline,
    Program, RedirectList, SimpleCommand, Word,
};
use brush_parser::word::{WordPiece, WordPieceWithSource};
use brush_parser::ParserOptions;

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
    let mut arithmetic = false;
    if parsed_targets(command, 0, &mut targets, &mut arithmetic).is_none() || arithmetic {
        // A part did not parse, or the parser found an arithmetic command, which can be
        // nested subshells read wrongly: `( (echo x > f) )` parses as `((...))`.
        // Scan the whole input, and keep what the parser found.
        fallback_targets(command, &mut targets);
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

/// Adds the targets that the parser finds in `command`. Returns `None` when a part of it,
/// such as a substitution body, does not parse or is too large to parse. Sets `arithmetic`
/// when it finds an arithmetic command.
fn parsed_targets(
    command: &str,
    depth: usize,
    out: &mut Vec<String>,
    arithmetic: &mut bool,
) -> Option<()> {
    if depth > MAX_SUBSTITUTION_DEPTH || too_large_to_parse(command) {
        return None;
    }
    let mut parser = brush_parser::Parser::new(
        std::io::BufReader::new(command.as_bytes()),
        &ParserOptions::default(),
    );
    let program = parser.parse_program().ok()?;
    Walk {
        depth,
        out,
        arithmetic,
    }
    .program(&program)
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

/// Walks the AST of one command. A method returns `None` when a part does not parse.
struct Walk<'a> {
    /// How many substitutions enclose this command.
    depth: usize,
    out: &'a mut Vec<String>,
    /// Set when an arithmetic command is found, so the caller also runs the fallback scan.
    arithmetic: &'a mut bool,
}

impl Walk<'_> {
    fn program(&mut self, program: &Program) -> Option<()> {
        program
            .complete_commands
            .iter()
            .try_for_each(|list| self.list(list))
    }

    fn list(&mut self, list: &CompoundList) -> Option<()> {
        for CompoundListItem(and_or, _) in &list.0 {
            self.pipeline(&and_or.first)?;
            for item in &and_or.additional {
                let (AndOr::And(pipeline) | AndOr::Or(pipeline)) = item;
                self.pipeline(pipeline)?;
            }
        }
        Some(())
    }

    fn pipeline(&mut self, pipeline: &Pipeline) -> Option<()> {
        pipeline
            .seq
            .iter()
            .try_for_each(|command| self.command(command))
    }

    fn command(&mut self, command: &Command) -> Option<()> {
        match command {
            Command::Simple(simple) => self.simple(simple),
            Command::Compound(compound, redirects) => {
                self.compound(compound)?;
                self.redirects(redirects.as_ref())
            }
            Command::Function(definition) => {
                let FunctionBody(compound, redirects) = &definition.body;
                self.compound(compound)?;
                self.redirects(redirects.as_ref())
            }
            Command::ExtendedTest(test, redirects) => {
                self.test(&test.expr)?;
                self.redirects(redirects.as_ref())
            }
        }
    }

    fn compound(&mut self, compound: &CompoundCommand) -> Option<()> {
        match compound {
            CompoundCommand::Arithmetic(a) => {
                *self.arithmetic = true;
                self.text(&a.expr.value).map(drop)
            }
            CompoundCommand::ArithmeticForClause(c) => {
                for expr in [&c.initializer, &c.condition, &c.updater]
                    .into_iter()
                    .flatten()
                {
                    self.text(&expr.value)?;
                }
                self.list(&c.body.list)
            }
            CompoundCommand::BraceGroup(b) => self.list(&b.list),
            CompoundCommand::Subshell(s) => self.list(&s.list),
            CompoundCommand::ForClause(f) => {
                for word in f.values.iter().flatten() {
                    self.word(word)?;
                }
                self.list(&f.body.list)
            }
            CompoundCommand::CaseClause(c) => {
                self.word(&c.value)?;
                for case in &c.cases {
                    for pattern in &case.patterns {
                        self.word(pattern)?;
                    }
                    if let Some(list) = &case.cmd {
                        self.list(list)?;
                    }
                }
                Some(())
            }
            CompoundCommand::IfClause(c) => {
                self.list(&c.condition)?;
                self.list(&c.then)?;
                for clause in c.elses.iter().flatten() {
                    if let Some(condition) = &clause.condition {
                        self.list(condition)?;
                    }
                    self.list(&clause.body)?;
                }
                Some(())
            }
            CompoundCommand::WhileClause(w) | CompoundCommand::UntilClause(w) => {
                self.list(&w.0)?;
                self.list(&w.1.list)
            }
            CompoundCommand::Coprocess(c) => self.command(&c.body),
        }
    }

    fn test(&mut self, expr: &ExtendedTestExpr) -> Option<()> {
        match expr {
            ExtendedTestExpr::And(a, b) | ExtendedTestExpr::Or(a, b) => {
                self.test(a)?;
                self.test(b)
            }
            ExtendedTestExpr::Not(e) | ExtendedTestExpr::Parenthesized(e) => self.test(e),
            ExtendedTestExpr::UnaryTest(_, word) => self.word(word).map(drop),
            ExtendedTestExpr::BinaryTest(_, left, right) => {
                self.word(left)?;
                self.word(right).map(drop)
            }
        }
    }

    fn simple(&mut self, simple: &SimpleCommand) -> Option<()> {
        // Prefix assignments such as `FOO=1 tee x` are not argv words.
        if let Some(prefix) = &simple.prefix {
            for item in &prefix.0 {
                self.item(item)?;
            }
        }
        let mut argv = Vec::new();
        if let Some(word) = &simple.word_or_name {
            argv.push(self.word(word)?);
        }
        if let Some(suffix) = &simple.suffix {
            for item in &suffix.0 {
                if let Some(text) = self.item(item)? {
                    argv.push(text);
                }
            }
        }
        analyze(&argv, self.out);
        Some(())
    }

    /// Handles redirects and process substitutions. Returns the text of argv words.
    fn item(&mut self, item: &CommandPrefixOrSuffixItem) -> Option<Option<String>> {
        match item {
            CommandPrefixOrSuffixItem::IoRedirect(redirect) => {
                self.redirect(redirect).map(|()| None)
            }
            CommandPrefixOrSuffixItem::ProcessSubstitution(_, sub) => {
                self.list(&sub.list).map(|()| None)
            }
            CommandPrefixOrSuffixItem::Word(word)
            | CommandPrefixOrSuffixItem::AssignmentWord(_, word) => self.word(word).map(Some),
        }
    }

    fn redirects(&mut self, redirects: Option<&RedirectList>) -> Option<()> {
        let Some(RedirectList(items)) = redirects else {
            return Some(());
        };
        items
            .iter()
            .try_for_each(|redirect| self.redirect(redirect))
    }

    fn redirect(&mut self, redirect: &IoRedirect) -> Option<()> {
        match redirect {
            IoRedirect::File(
                _,
                kind,
                IoFileRedirectTarget::Filename(word) | IoFileRedirectTarget::Duplicate(word),
            ) => {
                let target = self.word(word)?;
                if is_write(kind, &target) {
                    self.out.push(target);
                }
            }
            IoRedirect::File(_, _, IoFileRedirectTarget::ProcessSubstitution(_, sub)) => {
                self.list(&sub.list)?;
            }
            IoRedirect::File(_, _, IoFileRedirectTarget::Fd(_)) => {}
            // With an unquoted delimiter, bash runs the substitutions in the body.
            IoRedirect::HereDocument(_, doc) if doc.requires_expansion => {
                let body = &doc.doc.value;
                let pieces =
                    brush_parser::word::parse_heredoc(body, &ParserOptions::default()).ok()?;
                self.pieces(body, &pieces)?;
            }
            IoRedirect::HereDocument(..) => {}
            IoRedirect::HereString(_, word) => {
                self.word(word)?;
            }
            IoRedirect::OutputAndError(word, _) => {
                let target = self.word(word)?;
                self.out.push(target);
            }
        }
        Some(())
    }

    /// Runs the substitutions in `word` and returns its text after quote removal.
    fn word(&mut self, word: &Word) -> Option<String> {
        self.text(&word.value)
    }

    fn text(&mut self, raw: &str) -> Option<String> {
        let pieces = brush_parser::word::parse(raw, &ParserOptions::default()).ok()?;
        self.pieces(raw, &pieces)
    }

    /// Runs the substitutions in `pieces` and joins the pieces as bash does after quote
    /// removal. An expansion keeps its raw text, because its value is not known.
    fn pieces(&mut self, raw: &str, pieces: &[WordPieceWithSource]) -> Option<String> {
        let mut text = String::new();
        for piece in pieces {
            let source = raw.get(piece.start_index..piece.end_index)?;
            match &piece.piece {
                WordPiece::Text(s) | WordPiece::SingleQuotedText(s) => text.push_str(s),
                WordPiece::AnsiCQuotedText(s) => text.push_str(&ansi_c(s)),
                WordPiece::EscapeSequence(s) => {
                    // A backslash before a newline joins lines. It adds no character.
                    let escaped = s.strip_prefix('\\').unwrap_or(s);
                    if escaped != "\n" {
                        text.push_str(escaped);
                    }
                }
                WordPiece::DoubleQuotedSequence(inner)
                | WordPiece::GettextDoubleQuotedSequence(inner) => {
                    text.push_str(&self.pieces(raw, inner)?);
                }
                WordPiece::CommandSubstitution(body)
                | WordPiece::BackquotedCommandSubstitution(body) => {
                    parsed_targets(body, self.depth + 1, self.out, self.arithmetic)?;
                    text.push_str(source);
                }
                WordPiece::ParameterExpansion(_) | WordPiece::ArithmeticExpression(_) => {
                    // `${x:-$(cmd)}` and `$(( $(cmd) ))` run `cmd`.
                    let inner = source.get(1..)?;
                    if inner.contains(['$', '`']) {
                        self.text(inner)?;
                    }
                    text.push_str(source);
                }
                WordPiece::TildeExpansion(_) => text.push_str(source),
            }
        }
        Some(text)
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
mod tests;
