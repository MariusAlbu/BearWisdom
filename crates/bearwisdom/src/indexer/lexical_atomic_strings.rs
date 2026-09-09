//! Decode source string values without losing UTF-16 surrogate code units.
use super::{Forms, LitValue};

pub(super) fn decode(text: &str, forms: &Forms) -> Option<LitValue> {
    let mut chars = text.chars();
    let quote = chars.next().filter(|quote| forms.quotes.contains(quote))?;
    if chars.next_back()? != quote {
        return None;
    }
    let mut chars = chars.peekable();
    let mut units = Vec::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if ch == quote || ch == '\n' || ch == '\r' {
                return None;
            }
            units.extend(ch.encode_utf16(&mut [0; 2]).iter().copied());
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\n' => continue,
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }
            '\u{2028}' | '\u{2029}' => continue,
            '0' if !chars.peek().is_some_and(|c| c.is_ascii_digit()) => {
                units.push(0);
                continue;
            }
            _ => {}
        }
        if escaped == forms.unicode_escape {
            if forms.braced_unicode && chars.peek() == Some(&'{') {
                chars.next();
                let mut value = 0u32;
                let mut count = 0;
                loop {
                    let ch = chars.next()?;
                    if ch == '}' {
                        break;
                    }
                    value = value.checked_mul(16)?.checked_add(ch.to_digit(16)?)?;
                    count += 1;
                }
                if count == 0 || value > 0x10ffff {
                    return None;
                }
                if value <= 0xffff {
                    units.push(value as u16);
                } else {
                    units.push(0xd800 + ((value - 0x10000) >> 10) as u16);
                    units.push(0xdc00 + ((value - 0x10000) & 1023) as u16);
                }
            } else {
                units.push(hex(&mut chars, 4)?);
            }
        } else if escaped == forms.hex_escape {
            units.push(hex(&mut chars, 2)?);
        } else if let Some(&(_, value)) = forms.escapes.iter().find(|&&(ch, _)| ch == escaped) {
            units.push(value);
        } else if forms.identity_escapes && !escaped.is_ascii_digit() {
            units.extend(escaped.encode_utf16(&mut [0; 2]).iter().copied());
        } else {
            return None;
        }
    }
    Some(match String::from_utf16(&units) {
        Ok(text) => LitValue::Str(text),
        Err(_) => LitValue::Utf16(units),
    })
}

fn hex(chars: &mut impl Iterator<Item = char>, length: usize) -> Option<u16> {
    (0..length).try_fold(0u16, |value, _| {
        Some(value * 16 + chars.next()?.to_digit(16)? as u16)
    })
}

#[cfg(test)]
#[path = "lexical_atomic_strings_tests.rs"]
mod tests;
