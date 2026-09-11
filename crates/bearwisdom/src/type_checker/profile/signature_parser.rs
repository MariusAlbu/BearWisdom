//! Neutral, depth-aware signature and type-text parsing primitives.
//!
//! Language adapters select the layouts they support; these helpers do not
//! select a language or inspect source-language identities.

pub(crate) fn first_generic_arg(s: &str) -> Option<String> {
    let open = s.find('<')?;
    let close_rel = find_matching_bracket(&s[open..], '<', '>')?;
    let inner = &s[open + 1..open + close_rel];
    let mut depth = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let arg = inner[..i].trim();
                return (!arg.is_empty()).then(|| arg.to_string());
            }
            _ => {}
        }
    }
    let arg = inner.trim();
    (!arg.is_empty()).then(|| arg.to_string())
}

/// Find the index of the closing bracket that matches the first opening bracket
/// in `s`.  Uses depth counting so nested brackets are handled correctly.
///
/// Example: `find_matching_bracket("Map<K, List<V>>", '<', '>')` → `Some(14)`.
/// `s.find('>')` would incorrectly return `Some(11)` for this input.
pub(crate) fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Extract text after a plugin-selected depth-zero marker, stopping before a
/// depth-zero initializer. The marker itself is supplied by the adapter.
pub(crate) fn after_top_level_marker(s: &str, marker: char) -> Option<String> {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && ch == marker {
            let after = s[i + ch.len_utf8()..].trim();
            let ty = after[..initializer_split(after)].trim();
            return (!ty.is_empty()).then(|| ty.to_string());
        }
    }
    None
}

/// First depth-zero whitespace-delimited token. Adapters decide whether that
/// token denotes a return type in their source-language signatures.
pub(crate) fn first_top_level_token(s: &str) -> &str {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => return s[..i].trim(),
            _ => {}
        }
    }
    s.trim()
}

/// The structural location of a declared type inside a plugin-selected signature slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclaredTypeLayout {
    AfterMarker(char),
    Prefix,
    Postfix,
}

pub(crate) fn parse_declared_type_from_signature(
    sig: &str,
    layout: DeclaredTypeLayout,
) -> Option<String> {
    let trimmed = sig.trim();
    if trimmed.is_empty() {
        return None;
    }
    match layout {
        DeclaredTypeLayout::AfterMarker(marker) => return after_top_level_marker(trimmed, marker),
        DeclaredTypeLayout::Postfix => {
            // `name Type` — last whitespace-separated token.
            let mut depth: i32 = 0;
            let mut last_ws: Option<usize> = None;
            for (i, ch) in trimmed.char_indices() {
                match ch {
                    '<' | '[' | '(' | '{' => depth += 1,
                    '>' | ']' | ')' | '}' => depth -= 1,
                    c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
                    _ => {}
                }
            }
            last_ws.map(|ws| trimmed[ws + 1..].trim().to_string())
        }
        DeclaredTypeLayout::Prefix => {
            // `Type name` — everything before the last whitespace.
            let mut depth: i32 = 0;
            let mut last_ws: Option<usize> = None;
            for (i, ch) in trimmed.char_indices() {
                match ch {
                    '<' | '[' | '(' | '{' => depth += 1,
                    '>' | ']' | ')' | '}' => depth -= 1,
                    c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
                    _ => {}
                }
            }
            last_ws.map(|ws| trimmed[..ws].trim().to_string())
        }
    }
}

/// The byte offset where a depth-zero initializer begins.
fn initializer_split(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' | b']' | b')' | b'}' => depth -= 1,
            b'=' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    s.len()
}

/// The structural location of a parameter type within a plugin-selected slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParameterTypeLayout {
    AfterMarker(char),
    Prefix,
    Postfix,
}

/// Parse parameter types using a plugin-selected structural layout.  Leading
/// groups is data supplied by the adapter for syntaxes that put a receiver or
/// another non-parameter group before the real parameter list.
pub(crate) fn parse_parameter_types(
    sig: &str,
    layout: ParameterTypeLayout,
    leading_groups: usize,
) -> Option<Vec<String>> {
    let bytes = sig.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // Locate the param-list group's opening `(`: forward-scan for the
    // (groups_to_skip+1)-th `(` at depth 0 across `<` and `[` so generic /
    // index args don't fool us.
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut groups_skipped = 0usize;
    let mut open_idx: Option<usize> = None;
    let mut paren_depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' if depth_angle == 0 && depth_square == 0 && paren_depth == 0 => {
                if groups_skipped == leading_groups {
                    open_idx = Some(i);
                    break;
                }
                paren_depth += 1;
            }
            b'(' => paren_depth += 1,
            b')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    groups_skipped += 1;
                }
            }
            _ => {}
        }
    }
    let open = open_idx?;
    // Forward-scan from open to find the matching `)` at paren-depth 1.
    let mut depth_paren: i32 = 0;
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut close_idx: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth_paren += 1,
            b')' => {
                depth_paren -= 1;
                if depth_paren == 0 {
                    close_idx = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close_idx?;
    let inner = &sig[open + 1..close];
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    // Bracket-aware split on top-level commas.
    let mut parts: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let ibytes = inner.as_bytes();
    for (i, &b) in ibytes.iter().enumerate() {
        match b {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' | b']' | b')' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(inner[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        parts.push(inner[start..].to_string());
    }
    let types: Vec<String> = parts
        .into_iter()
        .map(|part| match layout {
            ParameterTypeLayout::AfterMarker(marker) => {
                extract_param_type_after_marker(&part, marker)
            }
            ParameterTypeLayout::Prefix => extract_param_type_prefix(&part),
            ParameterTypeLayout::Postfix => extract_param_type_postfix_no_colon(&part),
        })
        .filter(|s| !s.is_empty())
        .collect();
    Some(types)
}

/// Extract a parameter type after the plugin-selected marker.
fn extract_param_type_after_marker(part: &str, marker: char) -> String {
    let mut depth: i32 = 0;
    for (i, ch) in part.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ch if ch == marker && depth == 0 => {
                return part[i + ch.len_utf8()..].trim().to_string()
            }
            _ => {}
        }
    }
    part.trim().to_string()
}

/// Extract the suffix after the final depth-zero whitespace boundary.
fn extract_param_type_postfix_no_colon(part: &str) -> String {
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut depth: i32 = 0;
    let mut last_ws: Option<usize> = None;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
            _ => {}
        }
    }
    let ty = match last_ws {
        Some(ws) => trimmed[ws + 1..].trim(),
        None => trimmed,
    };
    ty.to_string()
}

/// Extract the prefix before the final depth-zero whitespace boundary.
fn extract_param_type_prefix(part: &str) -> String {
    // A default value (`Action<T>? configure = null`) is not part of the
    // parameter's type-name pair — strip it at the first depth-0 `=` before
    // taking the type prefix.
    let part = part[..initializer_split(part)].trim_end();
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut depth: i32 = 0;
    let mut last_ws: Option<usize> = None;
    let mut previous = '\0';
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
            _ => {}
        }
        previous = ch;
    }
    match last_ws {
        Some(ws) => trimmed[..ws].trim().to_string(),
        None => trimmed.to_string(),
    }
}

/// True when `t` is a plain (bare or dotted) type name — no generic args,
/// brackets, unions, spaces, pointers, or parameter lists. Used to decide when
/// a signature-derived return type is authoritative for the head over a
/// (possibly parameter) trailing TypeRef.
pub(crate) fn is_plain_type_name(t: &str) -> bool {
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}
