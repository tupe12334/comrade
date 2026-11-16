//! Conservative scan for Bash input that the parser cannot analyze.
//!
//! The text is split on shell separators (`;`, `&`, `|`, `(`, `)`, `` ` ``,
//! newlines), quotes are removed, and each piece is checked like a simple
//! command. Double-quoted text and heredoc bodies are not commands. When that
//! reading may hide code, the input is scanned again with quote marks
//! removed and all text read as code. The scan is one flat loop, so deep nesting cannot overflow
//! the stack. It can report extra targets, which is the safe error.

use std::iter::Peekable;
use std::str::Chars;

use super::shell_words::{ansi_c, is_fd_word};
use super::shell_writes::analyze;

/// Adds the write targets that a flat token scan finds in `command`.
pub(super) fn fallback_targets(command: &str, out: &mut Vec<String>) {
    if !scan(command, true, out) {
        // Double-quoted text holds a substitution, or a quote is not closed. The flat scan
        // cannot tell text from code there, so scan again with all quoted text read as code.
        scan(command, false, out);
    }
}

/// Scans `command` once. With `quotes_are_text`, quoted text and heredoc bodies are not
/// commands. Without it, quote marks are only removed. Returns false when the first reading
/// may hide code.
fn scan(command: &str, quotes_are_text: bool, out: &mut Vec<String>) -> bool {
    let mut scan = Scan {
        out,
        word: String::new(),
        in_word: false,
        argv: Vec::new(),
        redirect_next: false,
        dup_next: false,
        skip_next: false,
        heredoc_next: false,
        heredocs: Vec::new(),
    };
    let mut chars = command.chars().peekable();
    let mut single = false;
    let mut double = false;
    let mut sure = true;
    while let Some(c) = chars.next() {
        if single {
            if c == '\'' {
                single = false;
            } else {
                scan.push(c);
            }
            continue;
        }
        if double && !matches!(c, '"' | '\\') {
            if c == '`' || (c == '$' && chars.peek() == Some(&'(')) {
                sure = false;
            }
            scan.push(c);
            continue;
        }
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    scan.push(next);
                }
            }
            '$' if !double && chars.peek() == Some(&'\'') => {
                chars.next();
                let mut body = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\'' => break,
                        '\\' => {
                            body.push(c);
                            body.extend(chars.next());
                        }
                        _ => body.push(c),
                    }
                }
                scan.in_word = true;
                ansi_c(&body).chars().for_each(|c| scan.push(c));
            }
            '\'' if !double => {
                single = quotes_are_text;
                scan.in_word = true;
            }
            '"' => {
                double = quotes_are_text && !double;
                scan.in_word = true;
            }
            ' ' | '\t' if !double => scan.end_word(),
            '>' => {
                scan.start_redirect();
                match chars.peek().copied() {
                    Some('|') => {
                        chars.next();
                    }
                    Some('&') => {
                        // `>&1` and `>&-` name a descriptor; `>&file` writes the file.
                        chars.next();
                        scan.dup_next = true;
                    }
                    _ => {}
                }
            }
            '<' => {
                scan.drop_fd_digits();
                scan.end_word();
                if quotes_are_text && chars.next_if_eq(&'<').is_some() {
                    if chars.next_if_eq(&'<').is_some() {
                        scan.skip_next = true; // here-string
                    } else {
                        chars.next_if_eq(&'-');
                        scan.heredoc_next = true;
                    }
                } else {
                    scan.skip_next = true;
                }
            }
            '&' if chars.peek() == Some(&'>') => {}
            '\n' if quotes_are_text => {
                scan.end_command();
                for delimiter in std::mem::take(&mut scan.heredocs) {
                    skip_heredoc_body(&mut chars, &delimiter);
                }
            }
            '\n' | ';' | '&' | '|' | '(' | ')' | '`' => scan.end_command(),
            _ => scan.push(c),
        }
    }
    scan.end_command();
    sure && !single && !double
}

/// Consumes heredoc body lines up to and including the line that is `delimiter`.
fn skip_heredoc_body(chars: &mut Peekable<Chars>, delimiter: &str) {
    loop {
        let line: String = chars.by_ref().take_while(|&c| c != '\n').collect();
        if line.trim() == delimiter || chars.peek().is_none() {
            return;
        }
    }
}

struct Scan<'a> {
    out: &'a mut Vec<String>,
    word: String,
    in_word: bool,
    argv: Vec<String>,
    /// The next word is a redirect target.
    redirect_next: bool,
    /// The redirect is `>&`, so a descriptor target is not a file.
    dup_next: bool,
    /// The next word is an input redirect operand, which is not an argument.
    skip_next: bool,
    /// The next word is a heredoc delimiter.
    heredoc_next: bool,
    /// Delimiters of heredocs whose bodies start after the next newline.
    heredocs: Vec<String>,
}

impl Scan<'_> {
    fn push(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }

    fn drop_fd_digits(&mut self) {
        if self.in_word && self.word.bytes().all(|b| b.is_ascii_digit()) {
            self.word.clear();
            self.in_word = false;
        }
    }

    fn start_redirect(&mut self) {
        self.drop_fd_digits();
        self.end_word();
        self.redirect_next = true;
        self.dup_next = false;
        self.skip_next = false;
        self.heredoc_next = false;
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if std::mem::take(&mut self.heredoc_next) {
            self.heredocs.push(word);
        } else if self.redirect_next {
            self.redirect_next = false;
            if !(std::mem::take(&mut self.dup_next) && is_fd_word(&word)) {
                self.out.push(word);
            }
        } else if std::mem::take(&mut self.skip_next) {
            // Input redirect operand: not an argument.
        } else {
            self.argv.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        let assignments = self.argv.iter().take_while(|w| is_assignment(w)).count();
        analyze(&self.argv[assignments..], self.out);
        self.argv.clear();
        self.redirect_next = false;
        self.dup_next = false;
        self.skip_next = false;
        self.heredoc_next = false;
    }
}

/// True for `NAME=value` words, which are prefix assignments and not the program.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}
