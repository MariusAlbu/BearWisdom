//! F# type-definition extraction.
//!
//! Handles `type_definition` and emits Class/Struct/Enum/Interface/TypeAlias
//! symbols plus member symbols for union cases, enum cases, and record fields.

use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

use super::extract::{
    extract_class_inherits, extract_interface_implementation, first_identifier_text,
    qualify_with_parent, scope_path_from_parent, visit,
};

// ---------------------------------------------------------------------------
// Type definition
// ---------------------------------------------------------------------------

pub(super) fn extract_type_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // type_definition contains one of: anon_type_defn, record_type_defn,
    // union_type_defn, enum_type_defn, interface_type_defn, type_abbrev_defn,
    // type_extension, delegate_type_defn
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = match child.kind() {
            "anon_type_defn" => SymbolKind::Class,
            "record_type_defn" => SymbolKind::Struct,
            "union_type_defn" | "enum_type_defn" => SymbolKind::Enum,
            "interface_type_defn" => SymbolKind::Interface,
            "type_abbrev_defn" | "delegate_type_defn" => SymbolKind::TypeAlias,
            "type_extension" => SymbolKind::Class,
            _ => continue,
        };

        let name = extract_type_name(&child, src);
        if name.is_empty() {
            continue;
        }

        // Use the type_definition wrapper's start line so it matches the coverage
        // tool's node-counting (which records the type_definition node, not the body).
        let line = node.start_position().row as u32;
        let qualified_name = qualify_with_parent(&name, parent_index, symbols);
        let scope_path = scope_path_from_parent(parent_index, symbols);
        let idx = symbols.len();

        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind,
            visibility: Some(Visibility::Public),
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("type {}", name)),
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Walk members — emit child symbols for compound types and scan for refs
        match child.kind() {
            "union_type_defn" => {
                extract_union_cases(&child, src, symbols, Some(idx));
            }
            "enum_type_defn" => {
                extract_enum_cases(&child, src, symbols, Some(idx));
            }
            "record_type_defn" => {
                extract_record_fields(&child, src, symbols, Some(idx));
            }
            "anon_type_defn" => {
                // Scan the entire subtree for interface_implementation and
                // class_inherits_decl nodes, which may be deeply nested under
                // transparent supertype wrappers that are invisible to visit().
                collect_named_descendants(&child, "interface_implementation", |iface| {
                    extract_interface_implementation(iface, src, Some(idx), refs);
                });
                collect_named_descendants(&child, "class_inherits_decl", |inh| {
                    extract_class_inherits(inh, src, Some(idx), refs);
                });
            }
            _ => {}
        }
        visit(child, src, symbols, refs, Some(idx));
        break; // Only one body per type_definition
    }
}

/// Emit EnumMember symbols for each `union_type_case` descending from a node.
/// Grammar: `union_type_defn` → `union_type_cases` → `union_type_case`
fn extract_union_cases(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    collect_named_descendants(node, "union_type_case", |child| {
        // union_type_case: `| <identifier> [of <type>]`
        let name = first_identifier_text(child, src);
        if name.is_empty() { return; }
        let qualified_name = qualify_with_parent(&name, parent_index, symbols);
        let scope_path = scope_path_from_parent(parent_index, symbols);
        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::EnumMember,
            visibility: Some(Visibility::Public),
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });
    });
}

/// Emit EnumMember symbols for each `enum_type_case` descending from a node.
/// Grammar: `enum_type_defn` → `enum_type_cases` → `enum_type_case`
fn extract_enum_cases(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    collect_named_descendants(node, "enum_type_case", |child| {
        // enum_type_case: `| <identifier> = <int>`
        let name = first_identifier_text(child, src);
        if name.is_empty() { return; }
        let qualified_name = qualify_with_parent(&name, parent_index, symbols);
        let scope_path = scope_path_from_parent(parent_index, symbols);
        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::EnumMember,
            visibility: Some(Visibility::Public),
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });
    });
}

/// Emit Field symbols for each `record_field` descending from a node.
/// Grammar: `record_type_defn` → `record_fields` → `record_field`
fn extract_record_fields(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    collect_named_descendants(node, "record_field", |child| {
        // record_field: `[mutable] <identifier> : <type>`
        let name = first_identifier_text(child, src);
        if name.is_empty() { return; }
        let qualified_name = qualify_with_parent(&name, parent_index, symbols);
        let scope_path = scope_path_from_parent(parent_index, symbols);
        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Field,
            visibility: Some(Visibility::Public),
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });
    });
}

/// Walk the subtree of `node` and call `f` for every child whose kind matches `target_kind`.
/// Does not recurse into matched children (stops at the first match per branch).
fn collect_named_descendants<F>(node: &Node, target_kind: &str, mut f: F)
where
    F: FnMut(&Node),
{
    collect_named_descendants_inner(node, target_kind, &mut f);
}

fn collect_named_descendants_inner<F>(node: &Node, target_kind: &str, f: &mut F)
where
    F: FnMut(&Node),
{
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == target_kind {
            f(&child);
            // Don't recurse into the matched node — its children are sub-fields, not siblings
        } else {
            collect_named_descendants_inner(&child, target_kind, f);
        }
    }
}

fn extract_type_name(node: &Node, src: &str) -> String {
    // The grammar structure for type names:
    //   anon_type_defn / record_type_defn / union_type_defn / etc.
    //     type_name          ← a child node by KIND (not necessarily a named field)
    //       identifier       ← the actual name
    //
    // child_by_field_name("type_name") only works if the grammar declares it as
    // a named field. Walk children by kind to be grammar-agnostic.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_name" {
            // type_name → identifier (or long_identifier_or_op for generic types)
            let name = first_identifier_text(&child, src);
            if !name.is_empty() {
                return name;
            }
        }
    }
    // Fallback: direct identifier under the defn node
    first_identifier_text(node, src)
}
