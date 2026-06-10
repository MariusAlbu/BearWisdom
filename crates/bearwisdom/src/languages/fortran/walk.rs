// =============================================================================
// languages/fortran/walk.rs  —  Top-level tree-sitter traversal
//
// One big match on node.kind() dispatches each Fortran construct to either an
// inline emission (use_statement, subroutine_call, call_expression) or a named
// extractor in extractors.rs. The scope stacks (`locals`, `local_types`) are
// pushed/popped here so the call-emission branches can see them.
// =============================================================================

use super::extract::{
    collect_local_decls, collect_local_type_decls, is_fortran_callable_text, is_local,
    local_derived_type, push_sym, text,
};
use super::extractors::{
    emit_reexport_synthetics, extract_bound_procedures, extract_extends,
    extract_variable_declaration, find_child_name, find_derived_type_name, find_interface_name,
    find_module_name, find_program_name, find_submodule_name,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

pub(super) fn walk_node(
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
                    let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
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
                                            is_import_binding: false,
                                            is_reexport: false,
                                            source_symbol_index: sym_idx,
                                            target_name: sym_name,
                                            kind: EdgeKind::Imports,
                                            line: node.start_position().row as u32,
                                            col: 0,
                                            module: None, // filled in below once module_name is known
                                            chain: None,
                                            byte_offset: node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
                                            "identifier"
                                                if !local.is_empty() && source.is_empty() =>
                                            {
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
                                            is_import_binding: false,
                                            is_reexport: false,
                                            source_symbol_index: sym_idx,
                                            target_name: local,
                                            kind: EdgeKind::Imports,
                                            line: node.start_position().row as u32,
                                            col: 0,
                                            // module field holds the source symbol name for renames.
                                            // If there's no rename, this stays None.
                                            module: if source.is_empty() {
                                                None
                                            } else {
                                                Some(source)
                                            },
                                            chain: None,
                                            byte_offset: node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: sym_idx,
                    target_name: module_name.clone(),
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
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
                            let obj_text = sub_node
                                .named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            let method_text = sub_node
                                .named_child(count - 1)
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
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: sym_idx,
                                    target_name: method_text,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    col: 0,
                                    module: module_val,
                                    chain: None,
                                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        } else if count == 1 {
                            let name = sub_node
                                .named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                                refs.push(ExtractedRef {
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        }
                    }
                    _ => {
                        let name = text(sub_node, src);
                        if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                            refs.push(ExtractedRef {
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                col: 0,
                                module: None,
                                chain: None,
                                byte_offset: node.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
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
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                col: 0,
                                module: None,
                                chain: None,
                                byte_offset: node.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                            });
                        }
                    }
                    // derived_type_member_expression: obj%method
                    // named children: [0] = object, [last] = method name
                    "derived_type_member_expression" => {
                        let count = callee.named_child_count();
                        if count >= 2 {
                            let obj_text = callee
                                .named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            let method_text = callee
                                .named_child(count - 1)
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
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: sym_idx,
                                    target_name: method_text,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    col: 0,
                                    module: module_val,
                                    chain: None,
                                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        } else if count == 1 {
                            // Single named child — use as target_name, no module
                            let name = callee
                                .named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if is_fortran_callable_text(&name) && !is_local(&name, locals) {
                                refs.push(ExtractedRef {
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
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

pub(super) fn walk_children(
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
