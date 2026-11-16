//! Helpers for reading shell words, shared by the parser path and the fallback scan.

use std::iter::Peekable;
use std::str::Chars;

/// True for `-`, `N` and `N-`: the forms of a `>&` or `<&` target that name a descriptor.
/// Any other target of `>&` is a file that bash opens for writing.
pub(super) fn is_fd_word(word: &str) -> bool {
    let digits = word.strip_suffix('-').unwrap_or(word);
    word == "-" || (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Decodes the body of a `$'...'` string as bash does (see QUOTING in bash(1)).
/// An unknown escape keeps its backslash. A NUL byte ends the string, as in bash.
pub(super) fn ansi_c(body: &str) -> String {
    let mut out = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        let decoded = if c == '\\' {
            escape(&mut chars)
        } else {
            Decoded::Char(c)
        };
        match decoded {
            Decoded::Byte(0) | Decoded::Char('\0') => break,
            Decoded::Byte(byte) => out.push(byte),
            Decoded::Char(c) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            Decoded::Literal(text) => out.extend_from_slice(text.as_bytes()),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

enum Decoded {
    Byte(u8),
    Char(char),
    Literal(String),
}

/// Decodes one escape sequence. The backslash is already read.
fn escape(chars: &mut Peekable<Chars<'_>>) -> Decoded {
    let Some(e) = chars.next() else {
        return Decoded::Literal("\\".to_string());
    };
    match e {
        'a' => Decoded::Byte(0x07),
        'b' => Decoded::Byte(0x08),
        'e' | 'E' => Decoded::Byte(0x1b),
        'f' => Decoded::Byte(0x0c),
        'n' => Decoded::Byte(b'\n'),
        'r' => Decoded::Byte(b'\r'),
        't' => Decoded::Byte(b'\t'),
        'v' => Decoded::Byte(0x0b),
        '\\' | '\'' | '"' | '?' => Decoded::Char(e),
        '0'..='7' => {
            let (value, _) = digits(chars, 8, 2, e.to_digit(8).unwrap_or(0));
            // bash keeps the low byte of `\400` and above.
            Decoded::Byte((value & 0xff) as u8)
        }
        'x' | 'u' | 'U' => {
            let max = match e {
                'x' => 2,
                'u' => 4,
                _ => 8,
            };
            match digits(chars, 16, max, 0) {
                (_, 0) => Decoded::Literal(format!("\\{e}")),
                (value, _) if e == 'x' => Decoded::Byte(value as u8),
                (value, _) => char::from_u32(value)
                    .map_or_else(|| Decoded::Literal(format!("\\{e}")), Decoded::Char),
            }
        }
        'c' => match chars.next() {
            Some('?') => Decoded::Byte(0x7f),
            Some(x) if x.is_ascii() => Decoded::Byte(x.to_ascii_uppercase() as u8 & 0x1f),
            Some(x) => Decoded::Literal(format!("\\c{x}")),
            None => Decoded::Literal("\\c".to_string()),
        },
        other => Decoded::Literal(format!("\\{other}")),
    }
}

/// Reads up to `max` digits in `radix` onto `value`. Returns the value and the digit count.
fn digits(chars: &mut Peekable<Chars<'_>>, radix: u32, max: usize, mut value: u32) -> (u32, usize) {
    let mut count = 0;
    while count < max {
        let Some(digit) = chars.peek().and_then(|c| c.to_digit(radix)) else {
            break;
        };
        value = value * radix + digit;
        chars.next();
        count += 1;
    }
    (value, count)
}

#[cfg(test)]
mod tests {
    use super::{ansi_c, is_fd_word};

    #[test]
    fn decodes_every_ansi_c_escape() {
        assert_eq!(
            ansi_c(r"\a\b\e\E\f\n\r\t\v"),
            "\x07\x08\x1b\x1b\x0c\n\r\t\x0b"
        );
        assert_eq!(ansi_c(r#"\\\'\"\?"#), "\\'\"?");
        assert_eq!(ansi_c(r"\x41\x4aBC\x4"), "AJBC\x04");
        assert_eq!(ansi_c(r"\101\61x"), "A1x");
        assert_eq!(ansi_c(r"\u00e9\U0001F600"), "\u{e9}\u{1F600}");
        assert_eq!(ansi_c(r"\cA\ca\c?"), "\x01\x01\x7f");
    }

    #[test]
    fn keeps_unknown_and_incomplete_escapes() {
        assert_eq!(ansi_c(r"\q"), r"\q");
        assert_eq!(ansi_c(r"\xg"), r"\xg");
        assert_eq!(ansi_c(r"\c"), r"\c");
        assert_eq!(ansi_c("a\\"), "a\\");
    }

    #[test]
    fn nul_ends_the_string() {
        assert_eq!(ansi_c(r"a\0b"), "a");
        assert_eq!(ansi_c(r"a\x00b"), "a");
        assert_eq!(ansi_c(r"a\c@b"), "a");
    }

    #[test]
    fn descriptor_words() {
        for word in ["1", "12", "3-", "-"] {
            assert!(is_fd_word(word), "{word}");
        }
        for word in ["", "src/main.rs", "1x", "--", "-1"] {
            assert!(!is_fd_word(word), "{word}");
        }
    }
}
