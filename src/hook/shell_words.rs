//! Helpers for reading shell words, shared by the parser path and the fallback scan.

/// True for `-`, `N` and `N-`: the forms of a `>&` or `<&` target that name a descriptor.
/// Any other target of `>&` is a file that bash opens for writing.
pub(super) fn is_fd_word(word: &str) -> bool {
    let digits = word.strip_suffix('-').unwrap_or(word);
    word == "-" || (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}
