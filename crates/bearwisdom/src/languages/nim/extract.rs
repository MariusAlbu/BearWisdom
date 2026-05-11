// =============================================================================
// languages/nim/extract.rs  —  Nim extractor (no tree-sitter grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — `proc name(...)`, `func name(...)`, `template name(...)`,
//               `macro name(...)`, `iterator name(...)`, `converter name(...)`
//   Method    — `method name(...)`
//   Struct    — `type Name = object`
//   Enum      — `type Name = enum`
//   Interface — `type Name = concept`
//   TypeAlias — `type Name = OtherType` (not object/enum/concept)
//
// REFERENCES:
//   Imports   — `import module`, `import module, module2`, `from module import ...`
//               `include file`
//   Calls     — every `ident(` or `ident[...](...`  occurrence on any line
//               (direct call, template/macro invocation, type construction)
//   Calls     — method-call receiver: `obj.method(` → `method` ref
//               `a.b.c(` → `c` ref (terminal segment only; `a` / `b` are
//               field accesses whose type is unknown at line-scan time)
//
// Single-pass line scanner.  Nim's indentation-based syntax makes full block
// tracking impractical without a grammar, so declarations are detected only at
// their syntactic start lines; call extraction runs over every non-comment byte.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    // Track whether we're inside a type section (for type declarations).
    let mut in_type_section = false;
    // Indentation of the type section body (first indented line after `type`)
    let mut type_section_indent: usize = 0;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let indent = leading_spaces(raw);

        // Detect leaving a type section when indentation resets to ≤ section start.
        if in_type_section && indent <= type_section_indent && !trimmed.is_empty() {
            if indent == 0 {
                in_type_section = false;
            }
        }

        // Only process top-level declarations (indent == 0) or type section body.
        if indent == 0 {
            // Top-level keyword
            if trimmed == "type" || trimmed.starts_with("type ") && trimmed.ends_with(':') {
                in_type_section = true;
                type_section_indent = 0;
                i += 1;
                // Peek at the next non-blank line to determine body indent
                let mut j = i;
                while j < lines.len() && lines[j].trim().is_empty() {
                    j += 1;
                }
                if j < lines.len() {
                    type_section_indent = leading_spaces(lines[j]);
                }
                continue;
            }

            // Procedure / function / etc.
            if let Some((name, kind, vis)) = parse_proc_line(trimmed) {
                let start = i as u32;
                let end = find_block_end(&lines, i);
                symbols.push(make_sym(name, kind, vis, start, end));
                // Extract calls from the signature line and every body line.
                // Body lines are not visited by the main loop (we jump past
                // them with i = end + 1), so we scan them here explicitly.
                for body_line_idx in i..=end as usize {
                    extract_calls(lines[body_line_idx].trim(), body_line_idx as u32, &mut refs);
                }
                i = end as usize + 1;
                continue;
            }

            // import / from ... import
            if let Some(import_refs) = parse_import_line(trimmed, i as u32) {
                refs.extend(import_refs);
                i += 1;
                continue;
            }

            // include file
            if let Some(inc_ref) = parse_include_line(trimmed, i as u32) {
                refs.push(inc_ref);
                i += 1;
                continue;
            }

            // Single-line type declaration: `type Name = ...`
            if let Some((tname, tkind)) = parse_type_decl_line(trimmed) {
                let line = i as u32;
                symbols.push(make_sym(tname, tkind, Visibility::Public, line, line));
                i += 1;
                continue;
            }
        } else if in_type_section && indent >= type_section_indent && type_section_indent > 0 {
            // Inside a `type` section body — each indented `Name = ...` is a type.
            if let Some((tname, tkind)) = parse_type_section_entry(trimmed) {
                let line = i as u32;
                let end = find_type_block_end(&lines, i, indent);
                symbols.push(make_sym(tname, tkind, Visibility::Public, line, end));
                i = end as usize + 1;
                continue;
            }
        }

        // Call-site extraction: run on every non-comment, non-import line
        // regardless of indent level.  We skip lines that are pure declarations
        // (already handled above) and pure comment lines (filtered at the top).
        // `extract_calls` is additive — it emits nothing for lines without calls.
        extract_calls(trimmed, i as u32, &mut refs);

        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

/// Scan one logical source line for call-site references.
///
/// Nim call forms handled:
///   - `name(...)` or `name[params](...)` — direct call / type construction
///   - `receiver.method(...)` — UFCS / method dispatch; emits `method` as target
///   - `receiver.method[...](...)` — generic method call
///
/// Does NOT emit refs for:
///   - Nim keywords that look like calls (`if`, `while`, `case`, …)
///   - Single-letter identifiers (too noisy, rarely meaningful as call targets)
///   - Comment text after `#`
///   - String literal contents
fn extract_calls(line: &str, line_num: u32, out: &mut Vec<ExtractedRef>) {
    // Strip trailing comment.  Nim comments start with `#`.  String literals
    // can contain `#`, so we do a simple left-to-right scan that respects
    // balanced double-quoted strings (the most common case).  We do not handle
    // multi-line strings (`"""…"""`) here — those span lines and the scanner
    // already treats each line independently.
    let effective = strip_comment(line);

    let bytes = effective.as_bytes();
    let n = bytes.len();
    let mut pos = 0;

    while pos < n {
        // Skip whitespace and non-identifier starters.
        let b = bytes[pos];
        if b == b'"' {
            // Skip double-quoted string literal to avoid false positives inside strings.
            pos += 1;
            while pos < n {
                match bytes[pos] {
                    b'\\' => pos += 2, // escape
                    b'"' => { pos += 1; break; }
                    _ => pos += 1,
                }
            }
            continue;
        }
        if b == b'\'' {
            // Skip char literal.
            pos += 1;
            while pos < n && bytes[pos] != b'\'' {
                if bytes[pos] == b'\\' { pos += 1; }
                pos += 1;
            }
            if pos < n { pos += 1; }
            continue;
        }

        if !(b.is_ascii_alphabetic() || b == b'_') {
            pos += 1;
            continue;
        }

        // We are at the start of an identifier token.
        let id_start = pos;
        while pos < n && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
            pos += 1;
        }
        let ident = &effective[id_start..pos];

        // Skip past optional generic params `[...]` — they appear in
        // `foo[T](args)` and `obj.method[T](args)`.
        let after_generics = skip_generic_params(bytes, pos);

        // Check whether the identifier is followed immediately (or after
        // generic params) by `(`.
        let has_call_paren = after_generics < n && bytes[after_generics] == b'(';

        if has_call_paren && !is_nim_control_keyword(ident) && ident.len() > 1 {
            // Determine whether this call is the terminal segment of a
            // dot-chain.  Walk backward from id_start to see if the
            // preceding non-space character is `.`.
            let before = effective[..id_start].trim_end();
            let is_method_call = before.ends_with('.');

            // For a dot-chain call, `ident` is the method name — it is the
            // directly callable symbol, so emit it.  (The object before the
            // dot is a value expression whose type we don't track at scan time.)
            //
            // For a bare call `foo(...)`, `ident` is the callee directly.
            let target = ident.to_string();

            out.push(ExtractedRef {
                source_symbol_index: 0,
                target_name: target,
                kind: if is_method_call { EdgeKind::Calls } else { EdgeKind::Calls },
                line: line_num,
                module: None,
                chain: None,
                byte_offset: id_start as u32,
                namespace_segments: Vec::new(),
            });
        }

        // Advance past the `(` we matched (or just continue scanning).
        pos = after_generics;
        if pos < n && bytes[pos] == b'(' {
            pos += 1;
        }
    }
}

/// Returns the index just past any `[...]` generic params starting at `pos`.
/// Returns `pos` unchanged if the next byte is not `[`.
fn skip_generic_params(bytes: &[u8], mut pos: usize) -> usize {
    let n = bytes.len();
    if pos >= n || bytes[pos] != b'[' {
        return pos;
    }
    let mut depth = 0i32;
    while pos < n {
        match bytes[pos] {
            b'[' => { depth += 1; pos += 1; }
            b']' => {
                depth -= 1;
                pos += 1;
                if depth == 0 { break; }
            }
            _ => pos += 1,
        }
    }
    pos
}

/// Strip everything from the first unquoted `#` onward.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut pos = 0;
    while pos < n {
        match bytes[pos] {
            b'"' => {
                pos += 1;
                while pos < n {
                    match bytes[pos] {
                        b'\\' => pos += 2,
                        b'"' => { pos += 1; break; }
                        _ => pos += 1,
                    }
                }
            }
            b'\'' => {
                pos += 1;
                while pos < n && bytes[pos] != b'\'' {
                    if bytes[pos] == b'\\' { pos += 1; }
                    pos += 1;
                }
                if pos < n { pos += 1; }
            }
            b'#' => return &line[..pos],
            _ => pos += 1,
        }
    }
    line
}

/// Nim control-flow keywords that syntactically look like calls (they precede
/// a parenthesised expression) but are never callable symbols.
fn is_nim_control_keyword(s: &str) -> bool {
    matches!(
        s,
        "if" | "when" | "while" | "for" | "case" | "of" | "elif" | "else"
            | "try" | "except" | "finally" | "return" | "yield" | "break"
            | "continue" | "discard" | "raise" | "await" | "defer"
            | "and" | "or" | "not" | "in" | "notin" | "is" | "isnot"
            | "let" | "var" | "const" | "type" | "proc" | "func" | "method"
            | "template" | "macro" | "iterator" | "converter" | "import"
            | "from" | "include" | "export" | "block" | "do" | "bind"
            | "mixin" | "static"
    )
}

// ---------------------------------------------------------------------------
// Line parsers
// ---------------------------------------------------------------------------

/// Parse a proc/func/method/template/macro/iterator/converter line.
/// Returns (name, kind, visibility).
fn parse_proc_line(line: &str) -> Option<(String, SymbolKind, Visibility)> {
    let (is_pub, rest) = strip_pub(line);

    let (kind, rest) = if let Some(r) = rest.strip_prefix("proc ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("func ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("method ") {
        (SymbolKind::Method, r)
    } else if let Some(r) = rest.strip_prefix("template ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("macro ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("iterator ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("converter ") {
        (SymbolKind::Function, r)
    } else {
        return None;
    };

    let rest = rest.trim_start();
    // Name ends at '(' or ':' or '[' or whitespace — also strip * (export marker)
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }

    let vis = if is_pub { Visibility::Public } else { Visibility::Private };
    Some((name, kind, vis))
}

/// Parse top-level `type Name = ...` (single line, no section).
/// Returns (name, kind).
fn parse_type_decl_line(line: &str) -> Option<(String, SymbolKind)> {
    let rest = line.strip_prefix("type ")?;
    let rest = rest.trim_start();
    parse_type_rhs(rest)
}

/// Parse an entry inside a `type` section (indented `Name = ...`).
fn parse_type_section_entry(trimmed: &str) -> Option<(String, SymbolKind)> {
    // Skip lines that are continuations of a previous definition (start with `|` or whitespace).
    if trimmed.starts_with('|') || trimmed.starts_with('#') {
        return None;
    }
    parse_type_rhs(trimmed)
}

/// Parse `Name = rhs` → (name, kind).
fn parse_type_rhs(s: &str) -> Option<(String, SymbolKind)> {
    // Name may have generic params: `Name[T]` or `Name`
    let name: String = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    // Skip optional generic params, then look for `=`
    let after_name = s[name.len()..].trim_start();
    let after_name = if after_name.starts_with('[') {
        // Skip to matching `]`
        let mut depth = 0usize;
        let mut end = 0;
        for (pos, ch) in after_name.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = pos + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        after_name[end..].trim_start()
    } else {
        after_name
    };

    let rhs = after_name.strip_prefix('=')?.trim_start();

    let kind = if rhs.starts_with("object") || rhs.starts_with("ref object") || rhs.starts_with("ptr object") {
        SymbolKind::Struct
    } else if rhs.starts_with("enum") {
        SymbolKind::Enum
    } else if rhs.starts_with("concept") {
        SymbolKind::Interface
    } else if rhs.starts_with("tuple") {
        SymbolKind::Struct
    } else {
        SymbolKind::TypeAlias
    };

    Some((name, kind))
}

/// Parse `import` and `from ... import` lines.
/// Returns a list of `ExtractedRef` with `EdgeKind::Imports`.
///
/// Handled forms:
///   `import os, strformat`
///   `import std/sequtils`
///   `import std/[sequtils, strutils, options]`     ← bracketed group
///   `import pkg/foo/[bar, baz]`                    ← prefixed bracketed group
///   `from std/strformat import fmt`
///   `import other as O`
fn parse_import_line(line: &str, line_num: u32) -> Option<Vec<ExtractedRef>> {
    if let Some(rest) = line.strip_prefix("import ") {
        let names = expand_nim_imports(rest);
        if names.is_empty() { return None; }
        let modules: Vec<ExtractedRef> = names
            .into_iter()
            .map(|n| ExtractedRef {
                source_symbol_index: 0,
                target_name: n,
                kind: EdgeKind::Imports,
                line: line_num,
                module: None,
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
            })
            .collect();
        return Some(modules);
    }

    if let Some(rest) = line.strip_prefix("from ") {
        // `from module import symbol, symbol2` — `module` is typically a single
        // module path (`std/strformat`) but the bracketed group form is also
        // legal as the source. Reuse the same expander.
        let module_part = match rest.find("import") {
            Some(idx) => rest[..idx].trim(),
            None => rest.split_whitespace().next().unwrap_or(""),
        };
        if module_part.is_empty() { return None; }
        let names = expand_nim_imports(module_part);
        if names.is_empty() { return None; }
        let modules: Vec<ExtractedRef> = names
            .into_iter()
            .map(|n| ExtractedRef {
                source_symbol_index: 0,
                target_name: n,
                kind: EdgeKind::Imports,
                line: line_num,
                module: None,
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
            })
            .collect();
        return Some(modules);
    }

    None
}

/// Parse `include file` lines → single `Imports` ref per included path.
///
/// Nim `include` is textual inclusion; the included file's symbols become
/// visible in the including module's scope.  We model it as an `Imports` edge
/// so the resolver can walk it.
fn parse_include_line(line: &str, line_num: u32) -> Option<ExtractedRef> {
    let rest = line.strip_prefix("include ")?;
    let name = rest.trim().split_whitespace().next()?.to_string();
    if name.is_empty() { return None; }
    Some(ExtractedRef {
        source_symbol_index: 0,
        target_name: name,
        kind: EdgeKind::Imports,
        line: line_num,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    })
}

/// Expand a Nim import RHS into individual module paths. Splits on commas at
/// the top level, and expands `prefix/[a, b, c]` into `prefix/a`, `prefix/b`,
/// `prefix/c`. Strips trailing `as alias` clauses.
fn expand_nim_imports(rest: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut buf = String::new();
    for ch in rest.chars() {
        match ch {
            '[' => { depth += 1; buf.push(ch); }
            ']' => { depth -= 1; buf.push(ch); }
            ',' if depth == 0 => {
                expand_one_import(buf.trim(), &mut out);
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    expand_one_import(buf.trim(), &mut out);
    out
}

fn expand_one_import(item: &str, out: &mut Vec<String>) {
    let item = item.trim();
    if item.is_empty() { return; }
    // Strip `as alias` suffix.
    let item = item.split(" as ").next().unwrap_or(item).trim();
    // Bracket-group form: `prefix/[a, b]` — also handles `prefix / [a, b]`
    // where Nim allows spaces around the `/` separator.
    if let (Some(open), Some(close)) = (item.find('['), item.rfind(']')) {
        if open < close {
            // Normalize spaces around `/` inside the prefix, then strip the
            // trailing `/` that remains after the bracket.
            let prefix_raw = item[..open].trim().trim_end_matches('/').trim();
            let prefix = normalize_nim_path(prefix_raw);
            let inside = &item[open + 1..close];
            for sub in inside.split(',') {
                let sub = sub.trim();
                if sub.is_empty() { continue; }
                let sub = sub.split(" as ").next().unwrap_or(sub).trim();
                if prefix.is_empty() {
                    out.push(sub.to_string());
                } else {
                    out.push(format!("{prefix}/{sub}"));
                }
            }
            return;
        }
    }
    // Plain `name` or `prefix/name`. Nim allows `std / times` with spaces.
    out.push(normalize_nim_path(item));
}

/// Collapse `a / b / c` → `a/b/c` by stripping whitespace around `/`.
fn normalize_nim_path(s: &str) -> String {
    if !s.contains(" /") && !s.contains("/ ") {
        return s.to_string();
    }
    s.split('/').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strip_pub(s: &str) -> (bool, &str) {
    if let Some(r) = s.strip_prefix("pub ") {
        return (true, r.trim_start());
    }
    // Nim uses `*` after the name as an export marker, not `pub`.
    // We treat all proc/func as Public since visibility is post-name.
    (false, s)
}

fn make_sym(name: String, kind: SymbolKind, vis: Visibility, start: u32, end: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(vis),
        start_line: start,
        end_line: end,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Find where a block started at `start` ends.
/// In Nim, blocks end when indentation returns to the base level.
/// Simple heuristic: scan forward until a line at indent ≤ start's indent.
fn find_block_end(lines: &[&str], start: usize) -> u32 {
    let base_indent = leading_spaces(lines[start]);
    let mut end = start as u32;
    for (k, &line) in lines[start + 1..].iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let ind = leading_spaces(line);
        if ind <= base_indent {
            return end;
        }
        end = (start + 1 + k) as u32;
    }
    end
}

/// Like `find_block_end` but relative to a known `block_indent` level.
fn find_type_block_end(lines: &[&str], start: usize, block_indent: usize) -> u32 {
    let mut end = start as u32;
    for (k, &line) in lines[start + 1..].iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let ind = leading_spaces(line);
        if ind <= block_indent {
            return end;
        }
        end = (start + 1 + k) as u32;
    }
    end
}
