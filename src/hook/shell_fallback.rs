//! Conservative scan for Bash input that the parser cannot analyze.
//!
//! The text is split on shell separators (`;`, `&`, `|`, `(`, `)`, `` ` ``,
//! newlines), quotes are removed, and each piece is checked like a simple
//! command. The scan is one flat loop, so deep nesting cannot overflow the
//! stack. It can report extra targets, which is the safe error.

use super::shell_writes::analyze;

/// Adds the write targets that a flat token scan finds in `command`.
pub(super) fn fallback_targets(command: &str, out: &mut Vec<String>) {
    let mut scan = Scan {
        out,
        word: String::new(),
        in_word: false,
        argv: Vec::new(),
        redirect_next: false,
        skip_next: false,
    };
    let mut chars = command.chars().peekable();
    let mut single = false;
    let mut ansi = false;
    let mut double = false;
    while let Some(c) = chars.next() {
        if single {
            if c == '\'' {
                single = false;
            } else if ansi && c == '\\' {
                if let Some(next) = chars.next() {
                    scan.push(next);
                }
            } else {
                scan.push(c);
            }
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
                single = true;
                ansi = true;
                scan.in_word = true;
            }
            '\'' if !double => {
                single = true;
                ansi = false;
                scan.in_word = true;
            }
            '"' => {
                double = !double;
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
                        // `>&1` and `>&file`: the target starts with `&`, so it is skipped later.
                        chars.next();
                        scan.push('&');
                    }
                    _ => {}
                }
            }
            '<' => {
                scan.drop_fd_digits();
                scan.end_word();
                scan.skip_next = true;
            }
            '&' if chars.peek() == Some(&'>') => {}
            '\n' | ';' | '&' | '|' | '(' | ')' | '`' => scan.end_command(),
            _ => scan.push(c),
        }
    }
    scan.end_command();
}

struct Scan<'a> {
    out: &'a mut Vec<String>,
    word: String,
    in_word: bool,
    argv: Vec<String>,
    /// The next word is a redirect target.
    redirect_next: bool,
    /// The next word is an input redirect operand, which is not an argument.
    skip_next: bool,
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
        self.skip_next = false;
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if self.redirect_next {
            self.redirect_next = false;
            if !word.starts_with('&') {
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
        self.skip_next = false;
    }
}

/// True for `NAME=value` words, which are prefix assignments and not the program.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}
