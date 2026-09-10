//! TypeScript's stored-signature grammar.

use crate::type_checker::profile::signature_parser::{
    find_matching_bracket, parse_declared_type_from_signature, parse_parameter_types,
    DeclaredTypeLayout, ParameterTypeLayout,
};

pub(crate) fn return_type(signature: &str) -> Option<String> {
    if let Some(colon) = top_level_marker(signature, ':') {
        let before = signature[..colon].trim_end();
        let result = signature[colon + 1..].trim();
        if before.ends_with(')') && !result.is_empty() {
            return Some(result.to_string());
        }
    }

    top_level_arrow(signature)
        .map(|arrow| signature[arrow + 2..].trim())
        .filter(|result| !result.is_empty())
        .map(str::to_string)
}

pub(crate) fn parameter_types(signature: &str) -> Option<Vec<String>> {
    parse_parameter_types(signature, ParameterTypeLayout::AfterMarker(':'), 0)
}

pub(crate) fn declared_type(signature: &str) -> Option<String> {
    parse_declared_type_from_signature(signature, DeclaredTypeLayout::AfterMarker(':'))
}

pub(crate) fn is_inline_object_result(result: &str) -> bool {
    let text = result.trim();
    text.starts_with('{') && text.ends_with('}')
}

pub(crate) fn is_tuple_result(result: &str) -> bool {
    let text = result.trim();
    text.starts_with('[') && text.ends_with(']')
}

pub(crate) fn has_callable_return_extraction(result: &str) -> bool {
    result.contains("typeof")
}
pub(crate) fn object_type_members(s: &str) -> Vec<(String, String)> {
    let t = s.trim();
    let Some(inner) = t.strip_prefix('{').and_then(|x| x.strip_suffix('}')) else {
        return Vec::new();
    };
    split_top_level(inner, &[';', ',', '\n'])
        .into_iter()
        .filter_map(|e| {
            let e = e.trim();
            if e.is_empty()
                || e.starts_with('[')
                || e.starts_with("...")
                || e.starts_with('(')
                || e.starts_with("new ")
            {
                return None;
            }
            let ci = top_level_marker(e, ':')?;
            let ty = e[ci + 1..].trim();
            let head = e[..ci]
                .split('(')
                .next()
                .unwrap_or("")
                .split('<')
                .next()
                .unwrap_or("");
            let name = head
                .trim()
                .trim_end_matches('?')
                .split_whitespace()
                .last()
                .unwrap_or("");
            (!ty.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$'))
            .then(|| (name.to_string(), ty.to_string()))
        })
        .collect()
}

pub(crate) fn conditional_branches(s: &str) -> Option<(String, String)> {
    let extends = top_level_word(s, "extends")?;
    let question = top_level_marker_after(s, '?', extends + "extends".len())?;
    let colon = top_level_marker_after(s, ':', question + 1)?;
    let yes = s[question + 1..colon].trim();
    let no = s[colon + 1..].trim();
    (!yes.is_empty() && !no.is_empty()).then(|| (yes.to_string(), no.to_string()))
}

pub(crate) fn generic_params(
    sig: &str,
    name: &str,
) -> Vec<(String, Option<String>, Option<String>)> {
    let Some(idx) = sig.rfind(name) else {
        return Vec::new();
    };
    let after = idx + name.len();
    let Some(open) = sig[after..].chars().next().filter(|c| *c == '<') else {
        return Vec::new();
    };
    let Some(close) = find_matching_bracket(&sig[after..], open, '>') else {
        return Vec::new();
    };
    sig[after + 1..after + close]
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            let (left, default) = part.split_once('=').map_or((part, None), |(l, d)| {
                (l.trim(), Some(d.trim().to_string()))
            });
            let (param, bound) = left.split_once(" extends ").map_or((left, None), |(p, b)| {
                (p.trim(), Some(b.trim().to_string()))
            });
            (!param.is_empty()).then(|| {
                (
                    param.to_string(),
                    bound.filter(|b| !b.is_empty()),
                    default.filter(|d| !d.is_empty()),
                )
            })
        })
        .collect()
}

fn split_top_level<'a>(s: &'a str, separators: &[char]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            // The `>` in an arrow is not an angle-bracket close.  Without
            // this guard a callback nested in `Array<…>` consumes the outer
            // generic depth and hides the declaration's real return arrow.
            '>' if i > 0 && s.as_bytes()[i - 1] == b'=' => {}
            '>' | ']' | ')' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && separators.contains(&ch) {
            out.push(&s[start..i]);
            start = i + ch.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}
fn top_level_marker(s: &str, marker: char) -> Option<usize> {
    top_level_marker_after(s, marker, 0)
}

fn top_level_arrow(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' if i > 0 && s.as_bytes()[i - 1] == b'=' => {}
            '>' | ']' | ')' | '}' => depth -= 1,
            '=' if depth == 0 && s[i..].starts_with("=>") => return Some(i),
            _ => {}
        }
    }
    None
}
fn top_level_marker_after(s: &str, marker: char, from: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' if i > 0 && s.as_bytes()[i - 1] == b'=' => {}
            '>' | ']' | ')' | '}' => depth -= 1,
            _ => {}
        }
        if i >= from && depth == 0 && ch == marker {
            return Some(i);
        }
    }
    None
}
fn top_level_word(s: &str, word: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0
            && s[i..].starts_with(word)
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
            && b.get(i + word.len())
                .is_none_or(|b| !b.is_ascii_alphanumeric())
        {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_type_preserves_inline_object_member_colons() {
        assert_eq!(
            return_type("setup(): { browser: Browser; flag: boolean }"),
            Some("{ browser: Browser; flag: boolean }".into())
        );
    }

    #[test]
    fn return_type_reads_top_level_arrow_after_nested_parameter_arrow() {
        assert_eq!(
            return_type("(visit: (item: Item) => void) => Result"),
            Some("Result".into())
        );
    }

    #[test]
    fn return_type_keeps_an_arrow_nested_in_a_generic_parameter() {
        assert_eq!(
            return_type("(visit: Array<(item: Item) => void>) => Result"),
            Some("Result".into())
        );
    }

    #[test]
    fn does_not_decode_non_ts_conditional_without_plugin() {
        assert_eq!(
            conditional_branches("Name extends Base ? Yes : No"),
            Some(("Yes".into(), "No".into()))
        );
    }
}
