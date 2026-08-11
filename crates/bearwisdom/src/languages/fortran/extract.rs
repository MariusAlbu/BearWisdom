// =============================================================================
// languages/fortran/extract.rs — Fortran extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `subroutine` (name from `subroutine_statement.name` field)
//   Function  — `function`  (name from `function_statement.name` field)
//   Function  — `program`   (name from `program_statement.name` child)
//   Namespace — `module`    (name from `module_statement.name` child)
//   Namespace — `submodule` (name from `submodule_statement.name` child)
//   Struct    — `derived_type_definition` (name from `derived_type_statement`)
//   Variable  — `variable_declaration` at module/program/submodule scope
//
// REFERENCES:
//   Imports     — `use_statement` → `module_name` child
//   Calls       — `subroutine_call` → `subroutine` field
//   Calls       — `call_expression` → `function` field
//   Inherits    — `derived_type_statement` `base` field (EXTENDS clause)
// =============================================================================

use super::walk::walk_node;
use crate::types::{ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser};

/// Returns true when `name` looks like a real Fortran callable identifier
/// (subroutine, function, intrinsic). Filters out garbage tokens that
/// tree-sitter recovers into a callee position when the source contains
/// `.fypp` interpolation macros (`stdlib${ii}$_sgemv`) — the parser then
/// classifies the *first quoted argument* (`'TRANSPOSE'`, `'NO TRANSPOSE'`)
/// as the call target and produces a Calls ref to a string literal.
/// Also rejects names containing fypp interpolation markers (`$`) that
/// survive partial parsing as mangled template artifacts.
#[inline]
pub(super) fn is_fortran_callable_text(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.as_bytes()[0];
    // String literals (single-quoted Fortran character constants and
    // double-quoted variants) and numeric literals are never callables.
    if matches!(first, b'\'' | b'"') || first.is_ascii_digit() {
        return false;
    }
    // fypp template artifacts contain `$` — reject them so mangled names
    // like `optval_${t1[0]}$${k1}$` never become unresolvable Calls refs.
    !name.contains('$')
}

/// True when `name` is a Fortran statement keyword that the grammar has no
/// dedicated statement rule for, so it parses through the same production
/// as an ordinary bare function/subroutine call. `DEALLOCATE(x)` and
/// `NULLIFY(p)` are statements (Fortran 2018 §9.7, §9.8), but
/// tree-sitter-fortran 0.5's `_statements` choice has no `deallocate_statement`
/// or `nullify_statement` arm — unlike `allocate_statement`, which IS its own
/// rule — so both fall through to the bare `$.call_expression` alternative.
/// Checked case-insensitively (Fortran identifiers are case-insensitive)
/// only against a BARE callee name; `obj%deallocate()` invokes a real
/// user-defined type-bound procedure and is never checked against this.
#[inline]
pub(super) fn is_fortran_statement_keyword(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "deallocate" | "nullify")
}

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_fortran::LANGUAGE.into())
        .is_err()
    {
        return ExtractionResult::empty();
    }

    // fypp preprocessor directives (`#:include`, `#:set`, `#:for`, etc.) are
    // not valid Fortran and cause tree-sitter to produce ERROR recovery nodes.
    // When those ERROR nodes appear BEFORE a `module` statement, tree-sitter
    // sometimes produces the `module` node with zero children — the body is
    // swallowed by the preceding recovery. Blank out any line whose first
    // non-whitespace character is `#` before parsing so tree-sitter sees clean
    // Fortran. We blank in-place (same byte count) to preserve byte offsets
    // for line-number attribution; `${...}$` interpolation markers that
    // survive the blank are still filtered downstream by the `$` guards.
    let cleaned: String;
    let parse_src = if source.contains("#:") {
        // Build a new string where each '#'-prefixed line is replaced by spaces
        // of the same byte length (preserving line endings byte-for-byte).
        let mut out = String::with_capacity(source.len());
        for line in source.split_inclusive('\n') {
            // Line includes the trailing `\n`; check content without it.
            let trimmed = line.trim_start_matches(|c: char| c == ' ' || c == '\t');
            if trimmed.starts_with('#') {
                // Replace every non-newline byte with a space.
                for b in line.bytes() {
                    if b == b'\n' || b == b'\r' {
                        out.push(b as char);
                    } else {
                        out.push(' ');
                    }
                }
            } else {
                out.push_str(line);
            }
        }
        cleaned = out;
        cleaned.as_str()
    } else {
        source
    };

    let tree = match parser.parse(parse_src, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = parse_src.as_bytes();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let mut locals: Vec<HashSet<String>> = Vec::new();
    // Stack of var→derived-type maps, one entry per open scope. Used to
    // resolve `obj%method` calls to `method` with `module = type_name`
    // rather than `module = var_name`, enabling member lookup in the resolver.
    let mut local_types: Vec<HashMap<String, String>> = Vec::new();
    walk_node(
        tree.root_node(),
        src,
        &mut symbols,
        &mut refs,
        None,
        &mut locals,
        &mut local_types,
    );

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

/// Collect names declared by `variable_declaration` nodes inside the body
/// of a subroutine/function/program. Fortran array indexing (`mm(i, j)`)
/// uses identical syntax to function calls, so without this set the
/// extractor emits a false-positive `Calls` ref for every local-array
/// access — millions on numerical-library codebases.
pub(super) fn collect_local_decls(node: Node, src: &[u8], out: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "subroutine_statement" | "function_statement" => {
                collect_procedure_dummy_args(&text(child, src), out);
            }
            "associate_statement" => {
                collect_associate_locals(&text(child, src), out);
            }
            "variable_declaration" => {
                let mut vc = child.walk();
                for d in child.children(&mut vc) {
                    let name = match d.kind() {
                        "identifier" => text(d, src),
                        "init_declarator" => d
                            .child_by_field_name("left")
                            .map(|n| text(n, src))
                            .unwrap_or_default(),
                        "sized_declarator" => {
                            d.named_child(0).map(|n| text(n, src)).unwrap_or_default()
                        }
                        _ => continue,
                    };
                    if !name.is_empty() {
                        out.insert(name);
                    }
                }
            }
            // Don't recurse into nested function/subroutine — those have
            // their own scope and will get their own locals set when walked.
            "subroutine" | "function" | "program" | "module" | "submodule" => continue,
            _ => collect_local_decls(child, src, out),
        }
    }
}

fn collect_procedure_dummy_args(stmt: &str, out: &mut HashSet<String>) {
    if let Some(args) = first_balanced_paren(stmt) {
        for arg in split_top_level_commas(args) {
            let name = arg.trim();
            if is_fortran_identifier(name) {
                out.insert(name.to_string());
            }
        }
    }

    let lower = stmt.to_ascii_lowercase();
    if let Some(result_pos) = lower.find("result") {
        if let Some(result_name) = first_balanced_paren(&stmt[result_pos..]) {
            let name = result_name.trim();
            if is_fortran_identifier(name) {
                out.insert(name.to_string());
            }
        }
    }
}

fn collect_associate_locals(stmt: &str, out: &mut HashSet<String>) {
    let Some(bindings) = first_balanced_paren(stmt) else {
        return;
    };
    for binding in split_top_level_commas(bindings) {
        let name = binding.split("=>").next().unwrap_or("").trim();
        if is_fortran_identifier(name) {
            out.insert(name.to_string());
        }
    }
}

fn first_balanced_paren(text: &str) -> Option<&str> {
    let start = text.find('(')?;
    let mut depth = 0usize;
    for (rel, ch) in text[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return text.get(start + 1..start + rel);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn is_fortran_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Returns true if `name` is declared as a local variable in any open
/// scope on the stack. Inner scopes shadow outer per Fortran semantics,
/// but for filter purposes "any scope contains" is equivalent — a local
/// at any level disqualifies a Calls emission.
pub(super) fn is_local(name: &str, locals: &[HashSet<String>]) -> bool {
    locals
        .iter()
        .any(|s| s.iter().any(|local| local.eq_ignore_ascii_case(name)))
}

/// Collect `variable_name → derived_type_name` mappings from
/// `variable_declaration` nodes of the form `type(T) :: var1, var2, ...`
/// in the immediate body of a subroutine/function/program. Only derived
/// types are captured; intrinsic types (integer, real, etc.) are skipped
/// because they have no bound procedures to chain-walk.
pub(super) fn collect_local_type_decls(node: Node, src: &[u8], out: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_declaration" => {
                // Capture derived-type and class specifiers (`type(T)`, `class(T)`).
                // `derived_type` → name field is a `type_name` node.
                // `declared_type` → name field is an `identifier` node (`class(T)`).
                let type_name = child
                    .child_by_field_name("type")
                    .and_then(|tn| match tn.kind() {
                        "derived_type" => tn.child_by_field_name("name").map(|nn| text(nn, src)),
                        "declared_type" => tn.child_by_field_name("name").map(|nn| text(nn, src)),
                        _ => None,
                    });
                let Some(tname) = type_name else { continue };
                if tname.is_empty() || tname.contains('$') {
                    continue;
                }
                // Collect each declarator's variable name.
                let mut vc = child.walk();
                for d in child.children(&mut vc) {
                    let var_name = match d.kind() {
                        "identifier" => text(d, src),
                        "init_declarator" => d
                            .child_by_field_name("left")
                            .map(|n| text(n, src))
                            .unwrap_or_default(),
                        "sized_declarator" => {
                            d.named_child(0).map(|n| text(n, src)).unwrap_or_default()
                        }
                        _ => continue,
                    };
                    if !var_name.is_empty() {
                        out.insert(var_name.to_lowercase(), tname.clone());
                    }
                }
            }
            // Don't recurse into nested scopes — each has its own type map.
            "subroutine" | "function" | "program" | "module" | "submodule" => continue,
            _ => collect_local_type_decls(child, src, out),
        }
    }
}

/// Look up the derived type of `var_name` in the innermost scope that
/// declares it, searching the type-map stack from top (inner) to bottom.
pub(super) fn local_derived_type<'a>(
    var_name: &str,
    local_types: &'a [HashMap<String, String>],
) -> Option<&'a str> {
    let lower = var_name.to_lowercase();
    for scope in local_types.iter().rev() {
        if let Some(t) = scope.get(&lower) {
            return Some(t.as_str());
        }
    }
    None
}

/// Push a symbol into the output list, returning its index.
///
/// Returns `usize::MAX` (sentinel) without pushing when `name` contains fypp
/// interpolation markers (`$`) — those are template artifacts from partial
/// parsing of `.fypp` source, not real Fortran identifiers.
pub(super) fn push_sym(
    node: Node,
    name: String,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> usize {
    if name.contains('$') {
        return usize::MAX;
    }
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    idx
}

pub(super) fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}

/// Extract the first contiguous Fortran identifier token from a node's bytes.
/// In error-recovery mode tree-sitter may extend a `name` node's span to
/// cover multiple lines; this function stops at the first whitespace or
/// non-identifier character so we never capture garbage from adjacent lines.
pub(super) fn first_word(node: Node, src: &[u8]) -> String {
    let raw = node.utf8_text(src).unwrap_or("").trim_start();
    // A Fortran identifier: starts with a letter, continues with letters,
    // digits, or underscores — and is case-insensitive but we preserve source case.
    let end = raw
        .bytes()
        .take_while(|&b| b.is_ascii_alphanumeric() || b == b'_')
        .count();
    raw[..end].to_string()
}
