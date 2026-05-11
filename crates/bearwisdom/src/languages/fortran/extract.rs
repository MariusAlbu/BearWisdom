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

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
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
fn is_fortran_callable_text(name: &str) -> bool {
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
    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None, &mut locals, &mut local_types);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

/// Collect names declared by `variable_declaration` nodes inside the body
/// of a subroutine/function/program. Fortran array indexing (`mm(i, j)`)
/// uses identical syntax to function calls, so without this set the
/// extractor emits a false-positive `Calls` ref for every local-array
/// access — millions on numerical-library codebases.
fn collect_local_decls(node: Node, src: &[u8], out: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_declaration" => {
                let mut vc = child.walk();
                for d in child.children(&mut vc) {
                    let name = match d.kind() {
                        "identifier" => text(d, src),
                        "init_declarator" => d.child_by_field_name("left")
                            .map(|n| text(n, src))
                            .unwrap_or_default(),
                        "sized_declarator" => d.named_child(0)
                            .map(|n| text(n, src))
                            .unwrap_or_default(),
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

/// Returns true if `name` is declared as a local variable in any open
/// scope on the stack. Inner scopes shadow outer per Fortran semantics,
/// but for filter purposes "any scope contains" is equivalent — a local
/// at any level disqualifies a Calls emission.
fn is_local(name: &str, locals: &[HashSet<String>]) -> bool {
    locals.iter().any(|s| s.contains(name))
}

/// Collect `variable_name → derived_type_name` mappings from
/// `variable_declaration` nodes of the form `type(T) :: var1, var2, ...`
/// in the immediate body of a subroutine/function/program. Only derived
/// types are captured; intrinsic types (integer, real, etc.) are skipped
/// because they have no bound procedures to chain-walk.
fn collect_local_type_decls(node: Node, src: &[u8], out: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_declaration" => {
                // Capture derived-type and class specifiers (`type(T)`, `class(T)`).
                // `derived_type` → name field is a `type_name` node.
                // `declared_type` → name field is an `identifier` node (`class(T)`).
                let type_name = child.child_by_field_name("type").and_then(|tn| {
                    match tn.kind() {
                        "derived_type" => tn.child_by_field_name("name").map(|nn| text(nn, src)),
                        "declared_type" => tn.child_by_field_name("name").map(|nn| text(nn, src)),
                        _ => None,
                    }
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
                        "init_declarator" => d.child_by_field_name("left")
                            .map(|n| text(n, src))
                            .unwrap_or_default(),
                        "sized_declarator" => d.named_child(0)
                            .map(|n| text(n, src))
                            .unwrap_or_default(),
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
fn local_derived_type<'a>(var_name: &str, local_types: &'a [HashMap<String, String>]) -> Option<&'a str> {
    let lower = var_name.to_lowercase();
    for scope in local_types.iter().rev() {
        if let Some(t) = scope.get(&lower) {
            return Some(t.as_str());
        }
    }
    None
}

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &mut Vec<HashSet<String>>,
    local_types: &mut Vec<HashMap<String, String>>,
) {
    match node.kind() {
        "subroutine" => {
            let name = find_child_name(node, src, "subroutine_statement");
            let name = name.unwrap_or_default();
            let mut scope_locals = HashSet::new();
            let mut scope_types = HashMap::new();
            collect_local_decls(node, src, &mut scope_locals);
            collect_local_type_decls(node, src, &mut scope_types);
            locals.push(scope_locals);
            local_types.push(scope_types);
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                // idx == usize::MAX means the name contained fypp markers and was
                // not emitted; skip the body to avoid orphaned refs.
                if idx != usize::MAX {
                    walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                }
            } else {
                walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
            }
            locals.pop();
            local_types.pop();
        }
        "function" => {
            let name = find_child_name(node, src, "function_statement");
            let name = name.unwrap_or_default();
            let mut scope_locals = HashSet::new();
            let mut scope_types = HashMap::new();
            collect_local_decls(node, src, &mut scope_locals);
            collect_local_type_decls(node, src, &mut scope_types);
            locals.push(scope_locals);
            local_types.push(scope_types);
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                if idx != usize::MAX {
                    walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                }
            } else {
                walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
            }
            locals.pop();
            local_types.pop();
        }
        "program" => {
            // PROGRAM name ... END PROGRAM name — main entry point → Function
            let name = find_program_name(node, src);
            let name = name.unwrap_or_default();
            let mut scope_locals = HashSet::new();
            let mut scope_types = HashMap::new();
            collect_local_decls(node, src, &mut scope_locals);
            collect_local_type_decls(node, src, &mut scope_types);
            locals.push(scope_locals);
            local_types.push(scope_types);
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                if idx != usize::MAX {
                    walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                }
            } else {
                walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
            }
            locals.pop();
            local_types.pop();
        }
        "module" => {
            let name = find_module_name(node, src);
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                if idx != usize::MAX {
                    walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                    // Emit synthetic Function symbols for names that are re-exported
                    // publicly via `public :: local_name` where `local_name` arrived
                    // as a rename alias (`use M, only: local => source`).  Without
                    // these synthetics, callers that import the re-exported name from
                    // this module find no symbol in the index and fail resolution.
                    emit_reexport_synthetics(node, src, idx, symbols);
                }
            } else {
                walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
            }
        }
        "submodule" => {
            // SUBMODULE (ancestor[:parent]) name — scoped namespace
            let name = find_submodule_name(node, src);
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                if idx != usize::MAX {
                    walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                }
            } else {
                walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
            }
        }
        "derived_type_definition" => {
            let name = find_derived_type_name(node, src);
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name.clone(), SymbolKind::Struct, symbols, parent_idx);
                if idx != usize::MAX {
                    // Emit Inherits edge for EXTENDS(base_type) if present.
                    extract_extends(node, src, idx, refs);
                    // Emit bound procedure names as qualified Variable members
                    // (`type_name.method_name`) so members_of(type_name) resolves.
                    extract_bound_procedures(node, src, &name, idx, symbols);
                    walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                }
            } else {
                walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
            }
        }
        "interface" => {
            // Named generic interface: `interface moment ... end interface`.
            // Acts as a function alias / overload set — callers reference
            // `moment` and Fortran dispatches at runtime to one of the
            // type-specific procedures inside the block. Emit the generic
            // name so cross-file callers can resolve to it.
            //
            // Anonymous `interface ... end interface` blocks (without a
            // name) declare external procedure prototypes — their inner
            // function/subroutine statements are walked by the normal
            // recursion. Skip the symbol push for those.
            if let Some(name) = find_interface_name(node, src) {
                if !name.is_empty() {
                    let idx = push_sym(
                        node,
                        name,
                        SymbolKind::Function,
                        symbols,
                        parent_idx,
                    );
                    if idx != usize::MAX {
                        walk_children(node, src, symbols, refs, Some(idx), locals, local_types);
                    }
                    return;
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
        }
        "variable_declaration" => {
            // Emit Variable symbols only at module/program/submodule scope
            // (parent_idx points to a Namespace/Function entry point).
            // Skip inside subroutines/functions to avoid local variable noise.
            if let Some(sym_idx) = parent_idx {
                let sym_kind = symbols.get(sym_idx).map(|s| s.kind);
                if matches!(sym_kind, Some(SymbolKind::Namespace)) {
                    extract_variable_declaration(node, src, sym_idx, symbols, parent_idx);
                }
            }
            // No walk_children — variable_declaration has no nested scopes.
        }
        "use_statement" => {
            let sym_idx = parent_idx.unwrap_or(0);
            let mut module_name = String::new();
            let mut has_only_list = false;
            let mut only_refs: Vec<ExtractedRef> = Vec::new();

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "module_name" | "name" if module_name.is_empty() => {
                        module_name = text(child, src);
                    }
                    "included_items" => {
                        // `only: sym1, local_alias => source_name, ...`
                        has_only_list = true;
                        let mut ic = child.walk();
                        for item in child.children(&mut ic) {
                            match item.kind() {
                                "identifier" => {
                                    // Plain symbol name in `only:` list.
                                    let sym_name = text(item, src);
                                    if !sym_name.is_empty() {
                                        only_refs.push(ExtractedRef {
                                            source_symbol_index: sym_idx,
                                            target_name: sym_name,
                                            kind: EdgeKind::Imports,
                                            line: node.start_position().row as u32,
                                            module: None, // filled in below once module_name is known
                                            chain: None,
                                            byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                        });
                                    }
                                }
                                "use_alias" => {
                                    // `local_name => source_name` rename.
                                    // local_name child kind is "local_name" or "identifier" (grammar alias).
                                    // source_name child kind is "identifier".
                                    let mut local = String::new();
                                    let mut source = String::new();
                                    let mut ac = item.walk();
                                    for part in item.children(&mut ac) {
                                        match part.kind() {
                                            "local_name" | "identifier" if local.is_empty() => {
                                                local = text(part, src);
                                            }
                                            "identifier" if !local.is_empty() && source.is_empty() => {
                                                source = text(part, src);
                                            }
                                            _ => {}
                                        }
                                    }
                                    // Emit the rename: local_name is what callers use,
                                    // source is the actual name in the module.
                                    // Encode as: target_name = local, module = source
                                    // so the resolver can look up source in the module file.
                                    if !local.is_empty() {
                                        only_refs.push(ExtractedRef {
                                            source_symbol_index: sym_idx,
                                            target_name: local,
                                            kind: EdgeKind::Imports,
                                            line: node.start_position().row as u32,
                                            // module field holds the source symbol name for renames.
                                            // If there's no rename, this stays None.
                                            module: if source.is_empty() { None } else { Some(source) },
                                            chain: None,
                                            byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Always emit the module-level import (wildcard if no only: list).
            if !module_name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: module_name.clone(),
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                    namespace_segments: Vec::new(),
                });
            }

            // Emit per-symbol imports if an only: list was present.
            // These let the resolver match individual call refs to their source module.
            if has_only_list && !module_name.is_empty() {
                for mut r in only_refs {
                    // Fill in the module path (except for rename refs where
                    // `module` already holds the source symbol name — we need
                    // a separate field to carry both, so we use namespace_segments
                    // to store the module name for rename refs).
                    if r.module.is_none() {
                        r.module = Some(module_name.clone());
                    } else {
                        // Rename: module = source_name. Store module path in
                        // namespace_segments[0] so build_file_context can recover it.
                        r.namespace_segments = vec![module_name.clone()];
                    }
                    refs.push(r);
                }
            }
        }
        "subroutine_call" => {
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(sub_node) = node.child_by_field_name("subroutine") {
                match sub_node.kind() {
                    "derived_type_member_expression" => {
                        // `call obj%method(args)` — extract object and method
                        // from the member expression. Use the local type map to
                        // replace the object variable name with its declared type
                        // so the resolver can probe members_of(type_name) directly.
                        let count = sub_node.named_child_count();
                        if count >= 2 {
                            let obj_text = sub_node.named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            let method_text = sub_node.named_child(count - 1)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if is_fortran_callable_text(&method_text) {
                                // Prefer the declared type of the object over its
                                // variable name — enables members_of(type) lookup.
                                let module_val = if obj_text.is_empty() {
                                    None
                                } else {
                                    let resolved = local_derived_type(&obj_text, local_types)
                                        .map(|t| t.to_string())
                                        .unwrap_or_else(|| obj_text.clone());
                                    Some(resolved)
                                };
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: method_text,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    module: module_val,
                                    chain: None,
                                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                });
                            }
                        } else if count == 1 {
                            let name = sub_node.named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                });
                            }
                        }
                    }
                    _ => {
                        let name = text(sub_node, src);
                        if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                namespace_segments: Vec::new(),
                            });
                        }
                    }
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
        }
        "call_expression" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // call_expression = _expression REPEAT1(argument_list)
            // The grammar has no named field; the callee is the first child.
            // Fortran array indexing (`mm(i, j)`) parses as a call_expression
            // with an identifier callee — indistinguishable from a real
            // function call by syntax alone. Skip emission when the callee
            // matches a known local-variable declaration.
            if let Some(callee) = node.child(0) {
                match callee.kind() {
                    "identifier" => {
                        let name = text(callee, src);
                        if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                namespace_segments: Vec::new(),
                            });
                        }
                    }
                    // derived_type_member_expression: obj%method
                    // named children: [0] = object, [last] = method name
                    "derived_type_member_expression" => {
                        let count = callee.named_child_count();
                        if count >= 2 {
                            let obj_text = callee.named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            let method_text = callee.named_child(count - 1)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if is_fortran_callable_text(&method_text) {
                                let module_val = if obj_text.is_empty() {
                                    None
                                } else {
                                    let resolved = local_derived_type(&obj_text, local_types)
                                        .map(|t| t.to_string())
                                        .unwrap_or_else(|| obj_text.clone());
                                    Some(resolved)
                                };
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: method_text,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    module: module_val,
                                    chain: None,
                                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                });
                            }
                        } else if count == 1 {
                            // Single named child — use as target_name, no module
                            let name = callee.named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx, locals, local_types);
        }
    }
}

/// Find the `name` field within a named child of the given kind.
/// E.g., `find_child_name(subroutine_node, "subroutine_statement")` returns
/// the name of the subroutine.
fn find_child_name(node: Node, src: &[u8], child_kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == child_kind {
            if let Some(name_node) = child.child_by_field_name("name") {
                let n = first_word(name_node, src);
                if !n.is_empty() { return Some(n); }
            }
            // Fallback: first `name` child
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = first_word(gc, src);
                    if !n.is_empty() { return Some(n); }
                }
            }
        }
    }
    None
}

fn find_module_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_statement" {
            // In error-recovery mode tree-sitter may give the `name`
            // node an over-extended byte span that includes subsequent
            // lines. Extract only the first Fortran identifier token
            // from the raw bytes to avoid multi-line pollution.
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = first_word(gc, src);
                    if !n.is_empty() { return Some(n); }
                }
            }
        }
    }
    None
}

fn find_derived_type_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "derived_type_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "type_name" {
                    let n = first_word(gc, src);
                    if !n.is_empty() { return Some(n); }
                }
            }
        }
    }
    None
}

fn find_program_name(node: Node, src: &[u8]) -> Option<String> {
    // program_statement has a single `name` child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "program_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = first_word(gc, src);
                    if !n.is_empty() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Interface block name: `interface NAME ... end interface NAME`.
///
/// tree-sitter-fortran wraps the whole construct in an `interface` node.
/// The optional name lives on an `interface_statement` child whose
/// `name` field (or first `name`/`identifier` named child) carries the
/// generic. Anonymous `interface` (procedure-prototype declaration form)
/// has no name child — the caller skips the symbol push and walks the
/// inner function/subroutine declarations as normal.
fn find_interface_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interface_statement" {
            // Try the `name` field first (newer grammar versions).
            if let Some(name_node) = child.child_by_field_name("name") {
                let n = first_word(name_node, src);
                if !n.is_empty() {
                    return Some(n);
                }
            }
            // Fallback: first `name` or `identifier` named child.
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if matches!(gc.kind(), "name" | "identifier") {
                    let n = first_word(gc, src);
                    if !n.is_empty() {
                        return Some(n);
                    }
                }
            }
            return None;
        }
    }
    None
}

fn find_submodule_name(node: Node, src: &[u8]) -> Option<String> {
    // submodule_statement: `name` child is the submodule identifier;
    // `ancestor` field is the parent module name (not our symbol name).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "submodule_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = first_word(gc, src);
                    if !n.is_empty() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Emit Inherits edge(s) from `derived_type_statement.base` field (EXTENDS clause).
/// base_type_specifier has a single `identifier` child that is the base type name.
fn extract_extends(
    node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "derived_type_statement" {
            // Iterate `base` field children (base_type_specifier nodes)
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "base_type_specifier" {
                    // base_type_specifier → single identifier child
                    let mut c3 = gc.walk();
                    for ggc in gc.children(&mut c3) {
                        if ggc.kind() == "identifier" {
                            let base_name = text(ggc, src);
                            if !base_name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: base_name,
                                    kind: EdgeKind::Inherits,
                                    line: gc.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Extract Variable symbols from a variable_declaration node.
/// Iterates `declarator` field entries; handles `identifier` and `init_declarator`.
fn extract_variable_declaration(
    node: Node,
    src: &[u8],
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // declarator field values: identifier | init_declarator | sized_declarator | ...
        let var_name = match child.kind() {
            "identifier" => text(child, src),
            "init_declarator" => {
                // left field = identifier | sized_declarator | coarray_declarator
                child.child_by_field_name("left")
                    .map(|n| text(n, src))
                    .unwrap_or_default()
            }
            "sized_declarator" => {
                // first named child is the identifier
                child.named_child(0).map(|n| text(n, src)).unwrap_or_default()
            }
            _ => continue,
        };
        if var_name.is_empty() {
            continue;
        }
        symbols.push(ExtractedSymbol {
            qualified_name: var_name.clone(),
            name: var_name,
            kind: SymbolKind::Variable,
            visibility: Some(Visibility::Public),
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: parent_idx,
        });
        let _ = source_symbol_index; // used for scope association via parent_idx
    }
}

/// Collect all `local_name => source_name` rename aliases declared by
/// `use_statement` children of `module_node`.  Returns a map from the
/// local (call-site) name to the canonical source name.
fn collect_module_rename_aliases(module_node: Node, src: &[u8]) -> std::collections::HashMap<String, String> {
    let mut aliases: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut cursor = module_node.walk();
    for child in module_node.children(&mut cursor) {
        if child.kind() != "use_statement" {
            continue;
        }
        let mut cc = child.walk();
        for item in child.children(&mut cc) {
            if item.kind() != "included_items" {
                continue;
            }
            let mut ic = item.walk();
            for entry in item.children(&mut ic) {
                if entry.kind() != "use_alias" {
                    continue;
                }
                let mut local = String::new();
                let mut source = String::new();
                let mut ac = entry.walk();
                for part in entry.children(&mut ac) {
                    match part.kind() {
                        "local_name" | "identifier" if local.is_empty() => {
                            local = text(part, src);
                        }
                        "identifier" if !local.is_empty() && source.is_empty() => {
                            source = text(part, src);
                        }
                        _ => {}
                    }
                }
                if !local.is_empty() && !source.is_empty() {
                    aliases.insert(local, source);
                }
            }
        }
    }
    aliases
}

/// Collect identifiers listed in any `public_statement` children of `module_node`.
fn collect_public_names(module_node: Node, src: &[u8]) -> std::collections::HashSet<String> {
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cursor = module_node.walk();
    for child in module_node.children(&mut cursor) {
        if child.kind() != "public_statement" {
            continue;
        }
        let mut cc = child.walk();
        for item in child.children(&mut cc) {
            if item.kind() == "identifier" {
                let n = text(item, src);
                if !n.is_empty() {
                    names.insert(n);
                }
            }
        }
    }
    names
}

/// For each name that appears in a `public_statement` AND originated from a
/// rename alias (`use M, only: local => source`), emit a synthetic Function
/// symbol in the module scope.  This makes the re-exported alias visible as
/// a first-class symbol so that callers importing it by the local name can
/// resolve to it via the normal import-based resolution path.
fn emit_reexport_synthetics(
    module_node: Node,
    src: &[u8],
    module_sym_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let aliases = collect_module_rename_aliases(module_node, src);
    if aliases.is_empty() {
        return;
    }
    let public_names = collect_public_names(module_node, src);
    for (local_name, _source_name) in &aliases {
        // Only emit when explicitly declared public — implicit public modules
        // re-export everything, but that case is handled by wildcard import
        // resolution in resolve_common already.  The gap we're closing is the
        // explicit `public :: local_name` re-export of an aliased import.
        if !public_names.contains(local_name) {
            continue;
        }
        // Skip if already defined as a real symbol (e.g. a subroutine with the
        // same name appears in the module — no duplicate needed).
        let already_defined = symbols
            .iter()
            .any(|s| &s.name == local_name && s.parent_index == Some(module_sym_idx));
        if already_defined {
            continue;
        }
        symbols.push(ExtractedSymbol {
            qualified_name: local_name.clone(),
            name: local_name.clone(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: module_node.start_position().row as u32,
            end_line: module_node.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: Some(module_sym_idx),
        });
    }
}

/// Push a symbol into the output list, returning its index.
/// Emit bound procedure names from a `derived_type_definition`'s `contains`
/// block as `Variable` symbols parented to the type. Qualified names are
/// stored as `type_name.method_name` so `members_of(type_name)` in the
/// symbol index returns them and `type_name.method_name` qname lookups hit.
///
/// Grammar: `procedure_statement` → `declarator` field → `method_name` leaves
/// for plain bindings, `binding` nodes for aliased bindings
/// (`procedure :: new => table_new` → binding_name="new", method_name="table_new").
fn extract_bound_procedures(
    type_node: Node,
    src: &[u8],
    type_name: &str,
    type_sym_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = type_node.walk();
    for child in type_node.children(&mut cursor) {
        // The `contains` block of a derived type is wrapped in a
        // `derived_type_procedures` node; `procedure_statement` nodes live
        // inside that wrapper.  Walk one level deeper when encountered.
        if child.kind() == "derived_type_procedures" {
            let mut pc = child.walk();
            for proc_stmt in child.children(&mut pc) {
                if proc_stmt.kind() == "procedure_statement" {
                    emit_procedure_statement_members(
                        proc_stmt, src, type_name, type_sym_idx, symbols,
                    );
                }
            }
            continue;
        }
        if child.kind() != "procedure_statement" {
            continue;
        }
        emit_procedure_statement_members(child, src, type_name, type_sym_idx, symbols);
    }
}

/// Emit Variable member symbols for each bound procedure name declared in a
/// single `procedure_statement` node. Qualifies symbols as `type_name.name`
/// so `members_of(type_name)` finds them.
fn emit_procedure_statement_members(
    proc_stmt: Node,
    src: &[u8],
    type_name: &str,
    type_sym_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut dc = proc_stmt.walk();
    for decl in proc_stmt.children(&mut dc) {
        match decl.kind() {
            "method_name" => {
                // Plain `procedure :: method_name`
                let name = text(decl, src);
                if name.is_empty() || name.contains('$') {
                    continue;
                }
                let qname = format!("{type_name}.{name}");
                if !symbols.iter().any(|s| s.qualified_name == qname) {
                    symbols.push(ExtractedSymbol {
                        qualified_name: qname,
                        name,
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: decl.start_position().row as u32,
                        end_line: decl.end_position().row as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: None,
                        doc_comment: None,
                        scope_path: None,
                        parent_index: Some(type_sym_idx),
                    });
                }
            }
            "binding" => {
                // `procedure :: alias_name => real_proc` — emit the public alias name.
                let mut alias_name = String::new();
                let mut bc = decl.walk();
                for part in decl.children(&mut bc) {
                    if part.kind() == "binding_name" {
                        let mut bnc = part.walk();
                        for bn_child in part.children(&mut bnc) {
                            if bn_child.kind() == "identifier" {
                                alias_name = text(bn_child, src);
                                break;
                            }
                        }
                        break;
                    }
                }
                if alias_name.is_empty() || alias_name.contains('$') {
                    continue;
                }
                let qname = format!("{type_name}.{alias_name}");
                if !symbols.iter().any(|s| s.qualified_name == qname) {
                    symbols.push(ExtractedSymbol {
                        qualified_name: qname,
                        name: alias_name,
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: decl.start_position().row as u32,
                        end_line: decl.end_position().row as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: None,
                        doc_comment: None,
                        scope_path: None,
                        parent_index: Some(type_sym_idx),
                    });
                }
            }
            _ => {}
        }
    }
}

/// Returns `usize::MAX` (sentinel) without pushing when `name` contains fypp
/// interpolation markers (`$`) — those are template artifacts from partial
/// parsing of `.fypp` source, not real Fortran identifiers.
fn push_sym(
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
    });
    idx
}

fn walk_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &mut Vec<HashSet<String>>,
    local_types: &mut Vec<HashMap<String, String>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx, locals, local_types);
    }
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}

/// Extract the first contiguous Fortran identifier token from a node's bytes.
/// In error-recovery mode tree-sitter may extend a `name` node's span to
/// cover multiple lines; this function stops at the first whitespace or
/// non-identifier character so we never capture garbage from adjacent lines.
fn first_word(node: Node, src: &[u8]) -> String {
    let raw = node.utf8_text(src).unwrap_or("").trim_start();
    // A Fortran identifier: starts with a letter, continues with letters,
    // digits, or underscores — and is case-insensitive but we preserve source case.
    let end = raw
        .bytes()
        .take_while(|&b| b.is_ascii_alphanumeric() || b == b'_')
        .count();
    raw[..end].to_string()
}
