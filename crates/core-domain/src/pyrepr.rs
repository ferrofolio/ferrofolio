// SPDX-License-Identifier: AGPL-3.0-only

//! Python's `repr()` of a string, which error messages of the Python backend quote
//! values with.

use crate::python_printable::is_printable;

/// `repr()` of a Python string.
pub fn repr_str(text: &str) -> String {
    repr_str_limited(text, usize::MAX)
}

/// `%.{limit}R` in CPython's error messages: the first `limit` characters of the
/// `repr()`, built without the rest of a long string.
pub fn repr_str_limited(text: &str, limit: usize) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len().min(limit).saturating_add(2));
    let mut written = 0_usize;
    // Appends a piece up to the limit; false once the limit is reached.
    let mut push = |out: &mut String, piece: &str| {
        for character in piece.chars() {
            if written >= limit {
                return false;
            }
            out.push(character);
            written = written.saturating_add(1);
        }
        true
    };
    let mut buffer = [0_u8; 4];
    if !push(&mut out, quote.encode_utf8(&mut buffer)) {
        return out;
    }
    for character in text.chars() {
        let escaped: String;
        let piece: &str = match character {
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if c == quote && quote == '"' => "\\\"",
            c if c == quote => "\\'",
            c if is_printable(c) => c.encode_utf8(&mut buffer),
            c => {
                let code = u32::from(c);
                escaped = if code < 0x100 {
                    format!("\\x{code:02x}")
                } else if code < 0x1_0000 {
                    format!("\\u{code:04x}")
                } else {
                    format!("\\U{code:08x}")
                };
                &escaped
            }
        };
        if !push(&mut out, piece) {
            return out;
        }
    }
    push(&mut out, quote.encode_utf8(&mut buffer));
    out
}
