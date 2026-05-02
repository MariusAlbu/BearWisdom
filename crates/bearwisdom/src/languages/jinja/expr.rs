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
/// Pipe-filter chains (`x | upper | replace('a','b')`) are intentionally
/// truncated at the first top-level `|` — only the leading expression is
/// scanned. This matches the post-PR behavior of strip_pipe_filters in
/// the nunjucks embed module.
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
            i += 1;
            continue;
        }

        // Identifier head: ASCII letter or underscore. Numeric prefixes
        // are not identifiers.
        if !(b.is_ascii_alphabetic() || b == b'_') {
            i += 1;
            continue;
        }

        // Walk identifier chars, then optional `.<ident>` continuations.
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
            i = skip_dotted_continuation(bytes, i);
            continue;
        }

        // `is <test>` and `is not <test>`: the identifier after `is` (or
        // `is not`) is a Jinja test name, not a value reference. Common
        // tests: defined, undefined, none, sameas, divisibleby, equalto,
        // mapping, sequence, string, number, iterable. Skip emitting.
        if prev_ident == Some("is") || prev_ident == Some("not") {
            // Look at the prior 1-2 idents to confirm `is <head>` or
            // `is not <head>`. We only tracked one prior, but the most
            // common forms are covered.
            prev_ident = Some(head);
            i = skip_dotted_continuation(bytes, i);
            continue;
        }

        // Don't filter Jinja2/Ansible runtime globals here — emission
        // happens unconditionally and the resolver routes them to the
        // synthetic `jinja-runtime` ecosystem (see
        // `crates/bearwisdom/src/ecosystem/runtime_grammars.rs`). Any
        // hand-list at this layer would shadow the data-file index and
        // drift from upstream.
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
        // Consume `.<ident>` continuations so we don't re-emit the tail.
        i = skip_dotted_continuation(bytes, i);
    }
}

fn skip_dotted_continuation(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] == b'.' {
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
