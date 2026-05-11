//! Jinja2 expression scanner.
//!
//! The leading expression of a `{{ ... }}` block (the part before any `|`
//! filter chain) is parsed natively here — not routed through tree-sitter-
//! javascript like the legacy Nunjucks path. JS routing was a known wrong
//! abstraction: Jinja's pipe-filter operator is parsed as bitwise-OR by JS,
//! and Jinja built-in functions (`lookup`, `range`, etc.) get emitted as
//! phantom JS function calls that never resolve.
//!
//! This module emits a conservative subset:
//!   * dotted identifier chains (`user`, `user.name`, `dict.value.length`)
//!     → one TypeRef per chain, target_name = the head identifier
//!   * subscript chains stop at the first `[` (so `arr[0].foo` emits `arr`)
//!
//! Function-call and filter-call recognition is deferred to follow-up
//! sessions (see jinja module docs for the staged rollout).

use crate::types::{EdgeKind, ExtractedRef};

/// Scan a Jinja2 expression body and emit TypeRef refs for each
/// identifier-chain head it discovers. The `body` is the trimmed payload
/// inside `{{ ... }}` or the RHS of a `{% set/if/for ... %}` directive.
///
/// Pipe-filter chains are handled at two levels:
/// - Top-level pipes are cut by `trim_at_top_level_pipe`, so `x | upper`
///   only scans `x`.
/// - Nested pipes inside parens/brackets (`(x | filter)`) are tracked via
///   `after_pipe` — the first identifier after any `|` is a filter name and
///   is suppressed from TypeRef emission.
///
/// Subscript chains (`arr[0].field`) are consumed in full by
/// `skip_chain_continuation` so only the root variable is emitted.
pub fn scan_expression(
    body: &str,
    source_symbol_index: usize,
    line: u32,
    refs: &mut Vec<ExtractedRef>,
) {
    let leading = trim_at_top_level_pipe(body);
    if leading.is_empty() {
        return;
    }

    let bytes = leading.as_bytes();
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    let mut prev_ident: Option<&str> = None;
    // True when the most recent non-whitespace, non-identifier token was `|`
    // at any paren depth. The identifier that follows is a filter name, not a
    // value reference.
    let mut after_pipe = false;

    while i < bytes.len() {
        let b = bytes[i];

        // String literals: skip wholesale.
        if let Some(quote) = in_str {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == quote {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            in_str = Some(b);
            after_pipe = false;
            i += 1;
            continue;
        }

        // Track pipe at any depth to suppress the following filter name.
        if b == b'|' {
            // `||` is not a Jinja filter separator.
            if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                after_pipe = false;
                i += 2;
            } else {
                after_pipe = true;
                i += 1;
            }
            continue;
        }

        // Non-identifier, non-pipe byte: clear pipe state on structural chars.
        if !(b.is_ascii_alphabetic() || b == b'_') {
            if !b.is_ascii_whitespace() {
                // Any structural token other than whitespace ends filter
                // position — e.g. `(x | f)(y)` — the `(` after `f` means `f`
                // was already consumed; the next ident starts fresh.
                after_pipe = false;
            }
            i += 1;
            continue;
        }

        // Walk identifier chars.
        let start = i;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphanumeric() || c == b'_' {
                i += 1;
                continue;
            }
            break;
        }

        let head = &leading[start..i];

        // Skip Jinja2 reserved words and operator-keywords. These would
        // otherwise emit refs to `if`, `else`, `not`, etc.
        if is_jinja_keyword(head) {
            prev_ident = Some(head);
            after_pipe = false;
            i = skip_chain_continuation(bytes, i);
            continue;
        }

        // `is <test>` and `is not <test>`: the identifier after `is` (or
        // `is not`) is a Jinja test name, not a value reference.
        if prev_ident == Some("is") || prev_ident == Some("not") {
            prev_ident = Some(head);
            after_pipe = false;
            i = skip_chain_continuation(bytes, i);
            continue;
        }

        // Suppress filter names: identifiers that appear immediately after a
        // `|` token (at any paren depth) are filter names, not variable refs.
        // This covers both top-level `x | upper` (already truncated by
        // trim_at_top_level_pipe) and nested `(x | filter)` forms.
        if after_pipe {
            prev_ident = Some(head);
            after_pipe = false;
            i = skip_chain_continuation(bytes, i);
            continue;
        }

        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: head.to_string(),
            kind: EdgeKind::TypeRef,
            line,
            module: None,
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
        });

        prev_ident = Some(head);
        after_pipe = false;
        // Consume `.<ident>` and `[...].<ident>` continuations so we don't
        // re-emit tail segments as new refs.
        i = skip_chain_continuation(bytes, i);
    }
}

/// Consume `.ident` and `[...].ident` chain continuations after a chain
/// head so that property accesses and subscript accesses don't get emitted
/// as independent variable references.
///
/// Handles: `a.b.c`, `a[0].b`, `a[k].b[j].c`, `a["key"].b`.
/// Stops at any non-continuation token (`(`, `|`, operator, etc.).
fn skip_chain_continuation(bytes: &[u8], mut i: usize) -> usize {
    loop {
        if i >= bytes.len() {
            break;
        }
        match bytes[i] {
            // `.ident` — attribute access.
            b'.' => {
                let after_dot = i + 1;
                if after_dot >= bytes.len() {
                    break;
                }
                let c = bytes[after_dot];
                if !(c.is_ascii_alphabetic() || c == b'_') {
                    break;
                }
                i = after_dot + 1;
                while i < bytes.len() {
                    let c = bytes[i];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            // `[...]` — subscript access: consume until matching `]` then
            // loop to pick up any `.ident` that follows.
            b'[' => {
                i += 1;
                let mut depth: i32 = 1;
                let mut in_str: Option<u8> = None;
                while i < bytes.len() && depth > 0 {
                    let b = bytes[i];
                    if let Some(q) = in_str {
                        if b == b'\\' && i + 1 < bytes.len() {
                            i += 2;
                            continue;
                        }
                        if b == q {
                            in_str = None;
                        }
                    } else {
                        match b {
                            b'"' | b'\'' => in_str = Some(b),
                            b'[' => depth += 1,
                            b']' => depth -= 1,
                            _ => {}
                        }
                    }
                    i += 1;
                }
                // After the closing `]`, loop back — a `.ident` may follow.
            }
            _ => break,
        }
    }
    i
}

fn trim_at_top_level_pipe(body: &str) -> &str {
    let bytes = body.as_bytes();
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(quote) = in_str {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == quote {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => in_str = Some(b),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'|' if depth == 0 => {
                // Skip `||` (not a Jinja filter; rare but defensive).
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    i += 2;
                    continue;
                }
                return body[..i].trim();
            }
            _ => {}
        }
        i += 1;
    }
    body.trim()
}

fn is_jinja_keyword(word: &str) -> bool {
    matches!(
        word,
        "and" | "or" | "not" | "in" | "is" | "if" | "else" | "elif"
            | "true" | "false" | "True" | "False" | "none" | "None"
            | "for" | "endfor" | "block" | "endblock" | "extends"
            | "include" | "import" | "from" | "as" | "with" | "without"
            | "context" | "set" | "do" | "macro" | "endmacro" | "call"
            | "endcall" | "filter" | "endfilter" | "raw" | "endraw"
            | "trans" | "endtrans" | "pluralize" | "endpluralize"
            | "autoescape" | "endautoescape" | "scoped" | "required"
            | "recursive" | "loop"
    )
}
