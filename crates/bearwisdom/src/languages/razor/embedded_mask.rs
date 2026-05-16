//! Razor-expression masking inside `<script>` bodies. Replaces every
//! server-side Razor construct (`@expr`, `@(…)`, `@{…}`, `@*…*@`, `@@`)
//! with same-width whitespace so the JS/TS extractor stops emitting
//! ghost refs while byte offsets and line positions stay accurate.

use super::embedded_scan::find_subseq;

/// Replace every Razor construct inside a `<script>` body with same-width
/// whitespace so the JS/TS extractor stops emitting ghost refs for the
/// server-side identifiers Razor substitutes at render time.
///
/// Preserves the original length and newline positions so downstream line
/// numbers stay accurate. Handles:
///   - `@* comment *@`              → whitespace (newlines preserved)
///   - `@@`                         → two spaces (escape — not an expression)
///   - `@(expr)`                    → whitespace over the whole `(…)`
///   - `@{ block }`                 → whitespace over the whole `{…}`
///   - `@identifier.chain(args)`    → whitespace over the implicit expression,
///                                    including any immediately-following
///                                    `.member`, `[index]`, or `(args)` tails
pub(super) fn mask_razor_expressions_in_script(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut out: Vec<u8> = content.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        // `@@` — escape; not a Razor construct.
        if bytes.get(i + 1) == Some(&b'@') {
            mask_range(&mut out, i, i + 2);
            i += 2;
            continue;
        }
        // `@*...*@` — Razor comment.
        if bytes.get(i + 1) == Some(&b'*') {
            if let Some(end) = find_subseq(bytes, i + 2, b"*@") {
                mask_range(&mut out, i, end + 2);
                i = end + 2;
                continue;
            }
            // Unterminated — consume to end.
            mask_range(&mut out, i, bytes.len());
            break;
        }
        // `@(expr)` — explicit expression.
        if bytes.get(i + 1) == Some(&b'(') {
            if let Some(end) = match_balanced(bytes, i + 1, b'(', b')') {
                mask_range(&mut out, i, end);
                i = end;
                continue;
            }
            i += 1;
            continue;
        }
        // `@{ block }` — explicit code block (rare inside scripts but possible).
        if bytes.get(i + 1) == Some(&b'{') {
            if let Some(end) = match_balanced(bytes, i + 1, b'{', b'}') {
                mask_range(&mut out, i, end);
                i = end;
                continue;
            }
            i += 1;
            continue;
        }
        // `@identifier` — implicit expression. Walk the identifier then any
        // chain tails `.member`, `[index]`, `(args)`.
        let id_start = i + 1;
        let id_end = consume_razor_identifier(bytes, id_start);
        if id_end == id_start {
            // `@` followed by non-identifier; leave as-is.
            i += 1;
            continue;
        }
        let chain_end = consume_razor_chain(bytes, id_end);
        mask_range(&mut out, i, chain_end);
        i = chain_end;
    }
    // SAFETY: we only replaced ASCII-range bytes with 0x20 / 0x09 / preserved
    // existing bytes. UTF-8 validity is maintained.
    String::from_utf8(out).unwrap_or_else(|_| content.to_string())
}

/// Replace bytes in `out[start..end]` with ASCII spaces, keeping any
/// newlines, `\r`, or `\t` intact so line numbers remain accurate.
fn mask_range(out: &mut [u8], start: usize, end: usize) {
    let end = end.min(out.len());
    for b in &mut out[start..end] {
        if *b == b'\n' || *b == b'\r' || *b == b'\t' {
            continue;
        }
        *b = b' ';
    }
}

/// Consume an identifier starting at `start`. Returns the byte past the
/// identifier. Razor identifiers are `[A-Za-z_][A-Za-z0-9_]*`.
fn consume_razor_identifier(bytes: &[u8], start: usize) -> usize {
    let mut j = start;
    if j >= bytes.len() || !is_razor_id_start(bytes[j]) {
        return start;
    }
    j += 1;
    while j < bytes.len() && is_razor_id_cont(bytes[j]) {
        j += 1;
    }
    j
}

/// Consume chain tails after a Razor implicit expression's head identifier:
/// `.member`, `[index]`, `(args)`. Stops at the first byte that's not a
/// chain continuation.
fn consume_razor_chain(bytes: &[u8], start: usize) -> usize {
    let mut j = start;
    loop {
        match bytes.get(j).copied() {
            Some(b'.') => {
                let after = consume_razor_identifier(bytes, j + 1);
                if after == j + 1 {
                    // `.` not followed by identifier — stop, leave the `.`.
                    break;
                }
                j = after;
            }
            Some(b'[') => match match_balanced(bytes, j, b'[', b']') {
                Some(end) => j = end,
                None => break,
            },
            Some(b'(') => match match_balanced(bytes, j, b'(', b')') {
                Some(end) => j = end,
                None => break,
            },
            _ => break,
        }
    }
    j
}

fn is_razor_id_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_razor_id_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Match a balanced `open`/`close` pair starting at `start` (which must be
/// `open`). Honors `"..."` and `'...'` string literals (so a `)` inside a
/// string is ignored). Returns the byte position past the matching close.
fn match_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(start) != Some(&open) {
        return None;
    }
    let mut depth: i32 = 0;
    let mut j = start;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'"' || b == b'\'' {
            j = consume_string_literal(bytes, j, b);
            continue;
        }
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        }
        j += 1;
    }
    None
}

fn consume_string_literal(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut j = start + 1;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'\\' {
            j += 2;
            continue;
        }
        if b == quote {
            return j + 1;
        }
        j += 1;
    }
    j
}
