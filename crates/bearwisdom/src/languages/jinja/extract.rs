//! Jinja2 (`.jinja`, `.jinja2`, `.j2`) host extractor.
//!
//! Owns its own logic — does NOT delegate to nunjucks, because Jinja2's
//! semantics (filter chains, function-style globals like `lookup`, `range`,
//! Python-like attribute access) diverge from Nunjucks once you go beyond
//! `{% block %}` / `{% extends %}` / `{% include %}`.
//!
//! Extraction surface (PR-1 — foundation):
//!   * file-stem `Class` symbol
//!   * `{% block <name> %}`           → Field symbol
//!   * `{% extends "..." %}`          → Imports ref
//!   * `{% include "..." %}`          → Imports ref
//!   * `{% import/from "..." %}`      → Imports ref
//!   * `{{ <expr> }}` payload         → identifier-chain TypeRefs via expr.rs
//!
//! Deferred to follow-up sessions:
//!   * Filter calls (`{{ x | indent }}` → `indent` as a Calls ref against a
//!     synthetic Jinja2 stdlib module)
//!   * Function calls inside expressions (`lookup(...)`, `range(...)`)
//!   * Ansible-specific globals (`ansible_facts`, `inventory_hostname`)
//!   * `{% set var = expr %}` symbol declarations
//!   * `{% for loop_var in expr %}` scope-introducing symbols

use crate::languages::jinja::expr;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let file_name = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    let host_index = 0usize;

    let bytes = source.as_bytes();
    let mut line: u32 = 0;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }

        // `{# comment #}` — skip wholesale.
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'#' {
            let body_start = i + 2;
            if let Some(close) = find_close(bytes, body_start, b'#', b'}') {
                let consumed = &source[body_start..close];
                line += consumed.matches('\n').count() as u32;
                i = close + 2;
                continue;
            }
            i += 2;
            continue;
        }

        // `{% directive %}` — emit symbols/imports for known directives.
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'%' {
            let body_start = i + 2;
            let Some(close) = find_close(bytes, body_start, b'%', b'}') else {
                i += 2;
                continue;
            };
            if let Some(body) = source.get(body_start..close) {
                let consumed_lines = body.matches('\n').count() as u32;
                handle_directive(body, host_index, line, &mut symbols, &mut refs, &file_name);
                line += consumed_lines;
            }
            i = close + 2;
            continue;
        }

        // `{{ expr }}` — scan expression for identifier-chain refs.
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let body_start = i + 2;
            let Some(close) = find_close(bytes, body_start, b'}', b'}') else {
                i += 2;
                continue;
            };
            if let Some(body) = source.get(body_start..close) {
                let consumed_lines = body.matches('\n').count() as u32;
                let trimmed = body.trim().trim_start_matches('-').trim_end_matches('-').trim();
                expr::scan_expression(trimmed, host_index, line, &mut refs);
                line += consumed_lines;
            }
            i = close + 2;
            continue;
        }

        i += 1;
    }

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn handle_directive(
    body: &str,
    host_index: usize,
    line: u32,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    file_name: &str,
) {
    let trimmed = body
        .trim()
        .trim_start_matches('-')
        .trim_end_matches('-')
        .trim();

    if let Some(rest) = trimmed.strip_prefix("block ") {
        let name = rest.split_whitespace().next().unwrap_or("");
        if !name.is_empty() {
            symbols.push(ExtractedSymbol {
                name: name.to_string(),
                qualified_name: format!("{file_name}.{name}"),
                kind: SymbolKind::Field,
                visibility: Some(Visibility::Public),
                start_line: line,
                end_line: line,
                start_col: 0,
                end_col: 0,
                signature: Some(trimmed.to_string()),
                doc_comment: None,
                scope_path: Some(file_name.to_string()),
                parent_index: Some(host_index),
            });
        }
        return;
    }

    if let Some(rest) = trimmed.strip_prefix("extends ") {
        if let Some(name) = strip_quotes(rest.trim()) {
            refs.push(make_imports_ref(host_index, strip_extension(&name), line));
        }
        return;
    }
    if let Some(rest) = trimmed.strip_prefix("include ") {
        if let Some(name) = strip_quotes(rest.trim()) {
            refs.push(make_imports_ref(host_index, strip_extension(&name), line));
        }
        return;
    }
    if let Some(rest) = trimmed.strip_prefix("import ") {
        let tok = rest.split_whitespace().next().unwrap_or("");
        if let Some(name) = strip_quotes(tok.trim()) {
            refs.push(make_imports_ref(host_index, strip_extension(&name), line));
        }
        return;
    }
    if let Some(rest) = trimmed.strip_prefix("from ") {
        // `{% from "lib.j2" import macro_name %}`
        let tok = rest.split_whitespace().next().unwrap_or("");
        if let Some(name) = strip_quotes(tok.trim()) {
            refs.push(make_imports_ref(host_index, strip_extension(&name), line));
        }
        return;
    }

    // `{% for <bindings> in <expr> %}` — emit Variable symbols for the
    // loop-bound names so `{{ vm.name }}` inside the loop body resolves.
    // Bindings can be single (`for x in xs`), tuple (`for k, v in items()`),
    // or parenthesized (`for (k, v) in pairs`).
    if let Some(rest) = trimmed.strip_prefix("for ") {
        if let Some(in_idx) = find_top_level_in(rest) {
            let bindings = &rest[..in_idx];
            // Scan the binding list and emit a symbol for each bare name.
            for name in bindings.split(',') {
                let bare = name
                    .trim()
                    .trim_matches(|c: char| c == '(' || c == ')' || c.is_whitespace());
                if is_valid_jinja_ident(bare) {
                    symbols.push(make_local_var(bare, file_name, host_index, line, trimmed));
                }
            }
            // Scan the iterable (the part after `in `) for identifier refs.
            // `in_idx` points at the `i` of `in `, so skip 3 bytes.
            let iterable = &rest[in_idx + 3..];
            super::expr::scan_expression(iterable.trim(), host_index, line, refs);
        }
        return;
    }

    // `{% set name = expr %}` or `{% set name %}...{% endset %}` — emit a
    // Variable symbol so subsequent `{{ name }}` refs resolve. Tuple-set
    // (`{% set x, y = ... %}`) splits the LHS on commas.
    if let Some(rest) = trimmed.strip_prefix("set ") {
        let lhs = rest.split('=').next().unwrap_or(rest);
        for name in lhs.split(',') {
            let bare = name.trim();
            if is_valid_jinja_ident(bare) {
                symbols.push(make_local_var(bare, file_name, host_index, line, trimmed));
            }
        }
        // Scan the RHS for refs.
        if let Some(eq_idx) = rest.find('=') {
            let rhs = &rest[eq_idx + 1..];
            super::expr::scan_expression(rhs.trim(), host_index, line, refs);
        }
        return;
    }

    // `{% if <expr> %}` / `{% elif <expr> %}` — scan the condition for refs.
    if let Some(rest) = trimmed
        .strip_prefix("if ")
        .or_else(|| trimmed.strip_prefix("elif "))
    {
        super::expr::scan_expression(rest.trim(), host_index, line, refs);
        return;
    }

    // `{% macro name(args) %}` — emit a Function symbol and treat positional
    // params as local Variables.
    if let Some(rest) = trimmed.strip_prefix("macro ") {
        let (name, after) = split_at_paren(rest);
        if !name.is_empty() {
            symbols.push(ExtractedSymbol {
                name: name.to_string(),
                qualified_name: format!("{file_name}.{name}"),
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: line,
                end_line: line,
                start_col: 0,
                end_col: 0,
                signature: Some(trimmed.to_string()),
                doc_comment: None,
                scope_path: Some(file_name.to_string()),
                parent_index: Some(host_index),
            });
            for param in split_macro_params(after) {
                if is_valid_jinja_ident(&param) {
                    symbols.push(make_local_var(&param, file_name, host_index, line, trimmed));
                }
            }
        }
        return;
    }
}

fn make_local_var(
    name: &str,
    file_name: &str,
    host_index: usize,
    line: u32,
    sig: &str,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("{file_name}.{name}"),
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: Some(sig.to_string()),
        doc_comment: None,
        scope_path: Some(file_name.to_string()),
        parent_index: Some(host_index),
    }
}

/// Locate the position of the top-level ` in ` token in a `for` binding
/// header. Skips `in` tokens nested inside parens or strings (e.g.
/// `for x in items(default in='x')` — pathological but defensive).
fn find_top_level_in(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_str {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => in_str = Some(b),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0
            && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b',')
            && b == b'i'
            && bytes[i + 1] == b'n'
            && (bytes[i + 2] == b' ' || bytes[i + 2] == b'\t')
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn split_at_paren(s: &str) -> (&str, &str) {
    if let Some(p) = s.find('(') {
        (s[..p].trim(), &s[p + 1..])
    } else {
        let head = s.trim();
        (head, "")
    }
}

fn split_macro_params(rest: &str) -> Vec<String> {
    let close = rest.find(')').unwrap_or(rest.len());
    rest[..close]
        .split(',')
        .filter_map(|p| {
            let bare = p.split('=').next().unwrap_or(p).trim();
            if bare.is_empty() {
                None
            } else {
                Some(bare.to_string())
            }
        })
        .collect()
}

fn is_valid_jinja_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn make_imports_ref(source_idx: usize, target: String, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target,
        kind: EdgeKind::Imports,
        line,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}

fn find_close(bytes: &[u8], from: usize, a: u8, b: u8) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == a && bytes[i + 1] == b {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn strip_quotes(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() >= 2
        && (s.starts_with('"') && s.ends_with('"')
            || s.starts_with('\'') && s.ends_with('\''))
    {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

fn strip_extension(path: &str) -> String {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(path);
    let parent = p.parent().and_then(|p| p.to_str()).unwrap_or("");
    if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{}/{}", parent.replace('\\', "/"), stem)
    }
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}
