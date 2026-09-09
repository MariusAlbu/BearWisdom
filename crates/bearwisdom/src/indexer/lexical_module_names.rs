//! Quoted-name decoding at source ingestion, never during semantic lookup.
pub(super) fn decode(raw: &str) -> Option<String> {
    let quote = raw.chars().next()?;
    if !matches!(quote, '\'' | '"') || !raw.ends_with(quote) || raw.len() < 2 {
        return None;
    }
    let mut chars = raw[1..raw.len() - 1].chars().peekable();
    let mut units = Vec::new();
    while let Some(ch) = chars.next() {
        if ch == quote || matches!(ch, '\n' | '\r') {
            return None;
        }
        let ch = if ch == '\\' {
            match chars.next()? {
                '\n' => continue,
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    continue;
                }
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'v' => '\u{b}',
                '0' if !chars.peek().is_some_and(|c| c.is_ascii_digit()) => '\0',
                '0'..='9' => return None,
                'x' => char::from_u32(hex(&mut chars, 2)?)?,
                'u' => {
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let mut value = 0u32;
                        let mut count = 0;
                        loop {
                            let digit = chars.next()?;
                            if digit == '}' {
                                break;
                            }
                            value = value.checked_mul(16)?.checked_add(digit.to_digit(16)?)?;
                            count += 1;
                        }
                        if count == 0 {
                            return None;
                        }
                        char::from_u32(value)?
                    } else {
                        units.push(hex(&mut chars, 4)? as u16);
                        continue;
                    }
                }
                escaped => escaped,
            }
        } else {
            ch
        };
        units.extend_from_slice(ch.encode_utf16(&mut [0; 2]));
    }
    String::from_utf16(&units).ok()
}

fn hex(chars: &mut impl Iterator<Item = char>, count: usize) -> Option<u32> {
    (0..count).try_fold(0u32, |value, _| {
        Some(value * 16 + chars.next()?.to_digit(16)?)
    })
}

#[cfg(test)]
#[path = "lexical_module_names_tests.rs"]
mod tests;
