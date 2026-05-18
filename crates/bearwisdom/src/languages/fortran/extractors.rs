// =============================================================================
// languages/fortran/extractors.rs  —  Pure node-extracting helpers
//
// One function per Fortran construct: pull a name out of a parent node, emit
// a specific edge kind, or harvest a sub-list. No traversal logic — that
// lives in walk.rs.
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility,
};
use super::extract::{first_word, text};
use tree_sitter::Node;

/// Find the `name` field within a named child of the given kind.
/// E.g., `find_child_name(subroutine_node, "subroutine_statement")` returns
/// the name of the subroutine.
pub(super) fn find_child_name(node: Node, src: &[u8], child_kind: &str) -> Option<String> {
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

pub(super) fn find_module_name(node: Node, src: &[u8]) -> Option<String> {
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

pub(super) fn find_derived_type_name(node: Node, src: &[u8]) -> Option<String> {
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

pub(super) fn find_program_name(node: Node, src: &[u8]) -> Option<String> {
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
pub(super) fn find_interface_name(node: Node, src: &[u8]) -> Option<String> {
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

pub(super) fn find_submodule_name(node: Node, src: &[u8]) -> Option<String> {
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
pub(super) fn extract_extends(
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
                                    byte_offset: gc.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
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
pub(super) fn extract_variable_declaration(
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
pub(super) fn emit_reexport_synthetics(
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

/// Emit bound procedure names from a `derived_type_definition`'s `contains`
/// block as `Variable` symbols parented to the type. Qualified names are
/// stored as `type_name.method_name` so `members_of(type_name)` in the
/// symbol index returns them and `type_name.method_name` qname lookups hit.
///
/// Grammar: `procedure_statement` → `declarator` field → `method_name` leaves
/// for plain bindings, `binding` nodes for aliased bindings
/// (`procedure :: new => table_new` → binding_name="new", method_name="table_new").
pub(super) fn extract_bound_procedures(
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
