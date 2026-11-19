// SPDX-License-Identifier: AGPL-3.0-only

//! Python's notion of whitespace in strings, which the domain's validators use.

/// `str.isspace()` for one character: Unicode whitespace and also the separators
/// U+001C to U+001F, which Rust does not count as whitespace.
pub fn is_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1C}'..='\u{1F}').contains(&character)
}

/// `str.strip()`.
pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// `"".join(text.split())`: the text without any whitespace.
pub fn remove_whitespace(text: &str) -> String {
    text.chars()
        .filter(|character| !is_space(*character))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_follows_python() {
        assert_eq!(strip("\u{1C} a b\u{3000}\u{85}\n"), "a b");
        assert_eq!(strip("\u{200B}a"), "\u{200B}a");
        assert_eq!(remove_whitespace(" DE89\u{A0}3704\u{1F}00 "), "DE89370400");
    }
}
