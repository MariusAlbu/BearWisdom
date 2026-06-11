// =============================================================================
// languages/c_lang/type_refs.rs  —  parameter-type and full-CST type-ref sweep
// =============================================================================

use tree_sitter::Node;

use super::helpers::node_text;
use super::predicates;
use super::typerefs::emit_typerefs_for_type_descriptor;
use crate::types::{EdgeKind, ExtractedRef};

// ---------------------------------------------------------------------------
// Parameter type ref emission
// ---------------------------------------------------------------------------

/// Walk a function_definition's declarator chain to find parameter_list,
/// then emit TypeRef for each parameter's type_identifier.
pub(super) fn emit_param_type_refs(
    func_node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The parameter_list lives inside the function_declarator inside the
    // declarator field. We walk the declarator subtree to find it.
    if let Some(decl_node) = func_node.child_by_field_name("declarator") {
        emit_param_types_from_declarator(&decl_node, src, source_idx, refs);
    }
}

fn emit_param_types_from_declarator(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "parameter_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "parameter_declaration" {
                    if let Some(type_node) = child.child_by_field_name("type") {
                        emit_typerefs_for_type_descriptor(type_node, src, source_idx, refs);
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                emit_param_types_from_declarator(&child, src, source_idx, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-CST type-ref sweep
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit:
///   - TypeRef for every named `type_identifier` that is not a C/C++ builtin.
///   - A Calls ref for every `template_argument_list` (represents generic type usage).
///   - TypeRef for every `base_class_clause` — the inherits ref.
///   - TypeRef for every `sizeof_expression` argument type.
///
/// This sweep runs after the main extraction and ensures the coverage engine can
/// match all relevant ref-producing node kinds regardless of nesting depth.
///
/// Calling-convention / export-qualifier macros that tree-sitter parses as a
/// leading type token are dropped by a catalog-driven `retain` in the caller,
/// after this sweep — see `extract_with_file`.
pub(super) fn sweep_typerefs<'a>(
    node: Node<'a>,
    src: &[u8],
    default_sym_idx: usize,
    language: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty()
                    && !predicates::is_c_primitive_type(&name)
                    && !predicates::is_c_compiler_intrinsic(&name)
                    && !predicates::is_template_param(&name)
                {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: default_sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
                // type_identifier is a leaf — no children to recurse into.
            }
            "template_argument_list" => {
                // Recurse into children for nested type_identifiers, but do NOT
                // emit a synthetic "<template_args>" ref — that token can never
                // resolve and only inflates unresolved counts.
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
            "base_class_clause" if language != "c" => {
                // Emit Inherits refs for base class identifiers.
                let mut ic = child.walk();
                for base in child.children(&mut ic) {
                    match base.kind() {
                        "type_identifier" => {
                            let name = node_text(base, src);
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: default_sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: base.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: base.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        }
                        "base_class_specifier" => {
                            let mut bsc = base.walk();
                            for inner in base.children(&mut bsc) {
                                if inner.kind() == "type_identifier" {
                                    let name = node_text(inner, src);
                                    if !name.is_empty() {
                                        refs.push(ExtractedRef {
                                            is_import_binding: false,
                                            is_reexport: false,
                                            source_symbol_index: default_sym_idx,
                                            target_name: name,
                                            kind: EdgeKind::Inherits,
                                            line: inner.start_position().row as u32,
                                            col: 0,
                                            module: None,
                                            chain: None,
                                            byte_offset: inner.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
            "sizeof_expression" => {
                // Emit TypeRef for the argument type of sizeof.
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "type_descriptor" {
                        emit_typerefs_for_type_descriptor(inner, src, default_sym_idx, refs);
                    }
                }
                // The sweep will emit TypeRef for type_identifier children too.
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
            // Skip string/comment nodes that have no useful type info.
            "string_literal"
            | "comment"
            | "number_literal"
            | "char_literal"
            | "concatenated_string" => {}
            _ => {
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
        }
    }
}
