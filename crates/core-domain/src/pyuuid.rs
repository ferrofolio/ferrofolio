// SPDX-License-Identifier: AGPL-3.0-only

//! `uuid.UUID(text)` of Python, which the repositories apply to every id they read.

use crate::pyrepr::repr_str_limited;
use crate::unicode_digits::decimal_digit;

/// `str(uuid.UUID(text))`: the UUID in lowercase hex digits in groups of 8, 4, 4,
/// 4 and 12, or the message of Python's `ValueError`.
///
/// Python removes every `urn:` and `uuid:`, strips braces from both ends, removes
/// every hyphen, requires 32 characters and reads them with `int(text, 16)`, which
/// also takes surrounding whitespace, a sign, a `0x` prefix, single underscores
/// between digits and decimal digits of any script.
pub fn parse(text: &str) -> Result<String, String> {
    let hex = text.replace("urn:", "").replace("uuid:", "");
    let hex = hex
        .trim_matches(|character| character == '{' || character == '}')
        .replace('-', "");
    if hex.chars().count() != 32 {
        return Err("badly formed hexadecimal UUID string".to_owned());
    }
    let value = int_base16(&hex).ok_or_else(|| {
        format!(
            "invalid literal for int() with base 16: {}",
            repr_str_limited(&hex, 200)
        )
    })?;
    // A minus sign never reaches `int`, so the value is never negative, and 32
    // digits never reach 2**128.
    let digits = format!("{value:032x}");
    let group = |range: std::ops::Range<usize>| digits.get(range).unwrap_or_default();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        group(0..8),
        group(8..12),
        group(12..16),
        group(16..20),
        group(20..32)
    ))
}

/// `int(text, 16)` for a text of at most 32 characters, which fits 128 bits.
fn int_base16(text: &str) -> Option<u128> {
    // CPython turns non-ASCII whitespace into a space and decimal digits of any
    // script into ASCII digits before it parses; any other non-ASCII character is
    // invalid.
    let mut ascii = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else if crate::text::is_space(character) {
            ascii.push(' ');
        } else {
            ascii.push(char::from(b'0' + decimal_digit(character)?));
        }
    }
    let ascii_space =
        |character: char| matches!(character, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r');
    let trimmed = ascii.trim_matches(ascii_space);
    let (negative, unsigned) = match trimmed.as_bytes().first() {
        Some(b'+') => (false, trimmed.get(1..)?),
        Some(b'-') => (true, trimmed.get(1..)?),
        _ => (false, trimmed),
    };
    // A base prefix may be followed by one underscore.
    let digits = match unsigned.get(..2) {
        Some("0x" | "0X") => {
            let rest = unsigned.get(2..)?;
            rest.strip_prefix('_').unwrap_or(rest)
        }
        _ => unsigned,
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let mut value: u128 = 0;
    for character in digits.chars().filter(|character| *character != '_') {
        let digit = character.to_digit(16)?;
        value = value.checked_mul(16)?.checked_add(u128::from(digit))?;
    }
    if negative && value != 0 {
        return None;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ids_like_python() {
        let canonical = "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b";
        for spelling in [
            canonical,
            "0D3E7C1A5B2F4C8E9A1B2C3D4E5F6A7B",
            "{0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b}",
            "urn:uuid:0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b",
            "{{0d3e7c1a5b2f4c8e9a1b2c3d4e5f6a7b}}}",
            "0d3e-7c1a-5b2f-4c8e-9a1b-2c3d-4e5f-6a7b",
        ] {
            assert_eq!(parse(spelling).as_deref(), Ok(canonical), "{spelling}");
        }
        // Recorded from Python 3.14.
        let a31 = "a".repeat(31);
        assert_eq!(
            parse(&format!("+{a31}")).as_deref(),
            Ok("0aaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        );
        assert_eq!(
            parse(&format!(" {a31}")).as_deref(),
            Ok("0aaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        );
        assert_eq!(
            parse(&format!("0x{}", "a".repeat(30))).as_deref(),
            Ok("00aaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        );
        assert_eq!(
            parse(&"\u{663}".repeat(32)).as_deref(),
            Ok("33333333-3333-3333-3333-333333333333")
        );
        assert_eq!(
            parse(&"a_".repeat(16)),
            Err(format!(
                "invalid literal for int() with base 16: '{}'",
                "a_".repeat(16)
            ))
        );
        assert_eq!(
            parse(&format!("-{}", "0".repeat(31))),
            Err("badly formed hexadecimal UUID string".to_owned())
        );
        assert_eq!(
            parse(&format!("{}\n", "a".repeat(32))),
            Err("badly formed hexadecimal UUID string".to_owned())
        );
        assert_eq!(
            parse(&format!("{}g", "a".repeat(31))),
            Err(format!(
                "invalid literal for int() with base 16: '{}g'",
                "a".repeat(31)
            ))
        );
        assert_eq!(
            parse(""),
            Err("badly formed hexadecimal UUID string".to_owned())
        );
    }
}
