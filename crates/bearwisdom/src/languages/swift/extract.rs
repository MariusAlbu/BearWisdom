// =============================================================================
// parser/extractors/swift/mod.rs  —  Swift symbol and reference extractor
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use super::calls::extract_calls_from_body;
use super::decorators::{
    extract_decorators, extract_extension_conformances, extract_guard_bindings,
    extract_switch_patterns,
};
use super::helpers::find_child_by_kind;
use super::symbols::{
    extract_type_inheritance, handle_class_declaration, push_associatedtype, push_deinit,
    push_extension, push_function_decl, push_import, push_init, push_property, push_subscript,
    push_type_decl, push_typealias, recurse_into_body,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static SWIFT_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "struct_declaration",   name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",     name_field: "name" },
    ScopeKind { node_kind: "protocol_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_swift::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Swift grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, SWIFT_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

pub(super) fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                push_import(&child, src, symbols.len(), refs);
            }

            "class_declaration" => {
                // Capture the index before handle_class_declaration pushes anything.
                // push_type_decl inside it pushes the class as the first symbol, so
                // pre_len is the correct index for the class regardless of how many
                // nested symbols get pushed during recursion.
                let pre_len = symbols.len();
                handle_class_declaration(&child, src, scope_tree, symbols, refs, parent_index);
                // If a symbol was pushed it's at pre_len.
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }

            "struct_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Struct, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "enum_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                symbols::recurse_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "protocol_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "extension_declaration" => {
                let idx = push_extension(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extension_conformances(&child, src, sym_idx, refs);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_function_type_refs(&child, src, sym_idx, refs);
                    let body = child.child_by_field_name("body")
                        .or_else(|| find_child_by_kind(&child, "code_block"));
                    if let Some(b) = body {
                        extract_calls_from_body(&b, src, sym_idx, refs);
                    }
                }
            }

            // Protocol member declarations (no body — abstract requirements).
            "protocol_function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_function_type_refs(&child, src, sym_idx, refs);
                }
            }

            "protocol_property_declaration" => {
                let pre_len = symbols.len();
                push_property(&child, src, scope_tree, symbols, parent_index);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }

            "associatedtype_declaration" => {
                // Emit as TypeAlias — just push a simple symbol for the associatedtype.
                push_associatedtype(&child, src, scope_tree, symbols, parent_index);
            }

            // Both possible grammar node names for initializer declarations.
            "initializer_declaration" | "init_declaration" => {
                let idx = push_init(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    let body = child.child_by_field_name("body")
                        .or_else(|| find_child_by_kind(&child, "code_block"))
                        .or_else(|| find_child_by_kind(&child, "function_body"));
                    if let Some(b) = body {
                        extract_calls_from_body(&b, src, sym_idx, refs);
                    }
                }
            }

            "deinit_declaration" => {
                push_deinit(&child, src, scope_tree, symbols, parent_index);
            }

            "guard_statement" => {
                if let Some(sym_idx) = parent_index {
                    extract_guard_bindings(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "switch_statement" => {
                if let Some(sym_idx) = parent_index {
                    extract_switch_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "property_declaration" | "stored_property" | "variable_declaration"
            | "willSet_didSet_block" | "computed_property" => {
                let pre_len = symbols.len();
                push_property(&child, src, scope_tree, symbols, parent_index);
                let sym_idx = if symbols.len() > pre_len { pre_len } else { parent_index.unwrap_or(0) };
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, sym_idx, refs);
                }
                // Always extract type annotation refs — even if no symbol was pushed
                // (e.g. access-control filters) the TypeRef is still valuable.
                extract_function_type_refs(&child, src, sym_idx, refs);
                // Universal type_identifier scan to catch any remaining type refs
                // in annotation nodes not covered by extract_function_type_refs.
                calls::extract_all_type_identifiers_from_node(&child, src, sym_idx, refs);
                // Extract calls from the property initializer value or computed body.
                let body = child.child_by_field_name("value")
                    .or_else(|| find_child_by_kind(&child, "computed_property"))
                    .or_else(|| find_child_by_kind(&child, "code_block"));
                if let Some(b) = body {
                    calls::extract_calls_from_body(&b, src, sym_idx, refs);
                }
            }

            "typealias_declaration" => {
                push_typealias(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "subscript_declaration" => {
                let idx = push_subscript(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Recurse into the computed_property body for calls.
                    let body = find_child_by_kind(&child, "computed_property")
                        .or_else(|| find_child_by_kind(&child, "code_block"));
                    if let Some(b) = body {
                        extract_calls_from_body(&b, src, sym_idx, refs);
                    }
                }
            }

            // Call expressions outside a function body (e.g. computed property
            // defaults, top-level expressions, stored property initializers).
            "call_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_calls_from_body(&child, src, sym_idx, refs);
            }

            // Standalone attribute at scope level (not already attached to a
            // declaration handled above) — emit TypeRef to the attribute type.
            "attribute" => {
                let sym_idx = parent_index.unwrap_or(0);
                decorators::emit_single_attribute(&child, src, sym_idx, refs);
            }

            // user_type nodes that appear at statement/declaration level — extract type refs.
            // This catches generic type arguments and type parameters.
            "user_type" | "optional_type" | "array_type" | "dictionary_type" | "function_type" => {
                let sym_idx = parent_index.unwrap_or(0);
                calls::extract_type_ref_from_swift_type(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // inheritance_specifier and type_inheritance_clause at declaration level.
            "inheritance_specifier" | "type_inheritance_clause" | "inherited_type" => {
                let sym_idx = parent_index.unwrap_or(0);
                // Walk the inheritance specifier and emit TypeRef/Inherits for each type.
                let mut ic = child.walk();
                for inherited in child.children(&mut ic) {
                    match inherited.kind() {
                        "user_type" | "type_identifier" | "simple_identifier" => {
                            calls::extract_type_ref_from_swift_type(&inherited, src, sym_idx, refs);
                        }
                        "inheritance_specifier" | "inherited_type" => {
                            // Recurse one level deeper for nested specifiers.
                            let mut iic = inherited.walk();
                            for inner in inherited.children(&mut iic) {
                                match inner.kind() {
                                    "user_type" | "type_identifier" | "simple_identifier" => {
                                        calls::extract_type_ref_from_swift_type(&inner, src, sym_idx, refs);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // Universal scan to catch any remaining type_identifier nodes.
                calls::extract_all_type_identifiers_from_node(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // type_annotation at declaration level (type ascriptions).
            "type_annotation" => {
                let sym_idx = parent_index.unwrap_or(0);
                if let Some(type_node) = child.child_by_field_name("type")
                    .or_else(|| child.named_child(0))
                {
                    if type_node.kind() == "protocol_composition_type" {
                        calls::extract_protocol_composition_refs(&type_node, src, sym_idx, refs);
                    } else {
                        calls::extract_type_ref_from_swift_type(&type_node, src, sym_idx, refs);
                    }
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // type_identifier at declaration level — emit TypeRef and recurse for nested contexts.
            "type_identifier" | "simple_identifier" => {
                if let Some(sym_idx) = parent_index {
                    calls::extract_type_ref_from_swift_type(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // enum_class_body / enum_body encountered directly — recurse into all items.
            // Handles enums with methods, properties, and nested cases.
            "enum_class_body" | "enum_body" => {
                let mut ec = child.walk();
                for item in child.children(&mut ec) {
                    match item.kind() {
                        "enum_case_declaration" | "enum_entry" => {
                            // Enum cases — extract symbols via the standard path.
                            extract_node(item, src, scope_tree, symbols, refs, parent_index);
                        }
                        _ => {
                            // Methods, properties, nested types inside enum.
                            extract_node(item, src, scope_tree, symbols, refs, parent_index);
                            // Also scan for type_identifiers.
                            if let Some(sym_idx) = parent_index {
                                calls::extract_all_type_identifiers_from_node(&item, src, sym_idx, refs);
                            }
                        }
                    }
                }
            }

            "ERROR" | "MISSING" => {}

            _ => {
                // For any other node, always check for type_identifier children.
                // This ensures we catch type_identifiers in parameter lists, generic constraints,
                // type bounds, and other type-bearing contexts.
                if let Some(sym_idx) = parent_index {
                    calls::extract_all_type_identifiers_from_node(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function type reference extraction
// ---------------------------------------------------------------------------

/// Walk the parameters and return type of a function / protocol function
/// declaration and emit TypeRef edges for all named types found.
///
/// Swift grammar (tree-sitter-swift 0.7.1):
///   `function_declaration` has `parameter` as direct named children,
///   and `return_type` as a field (not a `function_return_type` child node).
fn extract_function_type_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Return type is stored in the `return_type` field.
    if let Some(ret_type) = node.child_by_field_name("return_type") {
        if ret_type.kind() == "protocol_composition_type" {
            calls::extract_protocol_composition_refs(&ret_type, src, source_symbol_index, refs);
        } else {
            calls::extract_type_ref_from_swift_type(&ret_type, src, source_symbol_index, refs);
        }
    }

    // Parameters are direct named children of kind `parameter`.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "parameter" => {
                extract_param_type_refs(&child, src, source_symbol_index, refs);
            }
            // Fallback: older grammar wraps parameters in a clause.
            "parameter_clause" | "function_value_parameters" => {
                let mut pc = child.walk();
                for param in child.children(&mut pc) {
                    match param.kind() {
                        "parameter" | "function_value_parameter" | "optional_parameter" => {
                            extract_param_type_refs(&param, src, source_symbol_index, refs);
                        }
                        _ => {}
                    }
                }
            }
            // Direct type annotation at declaration level (protocol property, etc.)
            "type_annotation" => {
                if let Some(type_node) = child.child_by_field_name("type")
                    .or_else(|| child.child_by_field_name("name"))
                {
                    if type_node.kind() == "protocol_composition_type" {
                        calls::extract_protocol_composition_refs(&type_node, src, source_symbol_index, refs);
                    } else {
                        calls::extract_type_ref_from_swift_type(&type_node, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_param_type_refs(
    param: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The parameter's type is in the `type` field (tree-sitter-swift 0.7.1).
    if let Some(type_node) = param.child_by_field_name("type") {
        if type_node.kind() == "protocol_composition_type" {
            calls::extract_protocol_composition_refs(&type_node, src, source_symbol_index, refs);
        } else {
            calls::extract_type_ref_from_swift_type(&type_node, src, source_symbol_index, refs);
        }
        return;
    }

    // Fallback: scan named children.
    let mut cursor = param.walk();
    for child in param.children(&mut cursor) {
        match child.kind() {
            "type_annotation" => {
                if let Some(type_node) = child.child_by_field_name("type")
                    .or_else(|| child.child_by_field_name("name"))
                {
                    if type_node.kind() == "protocol_composition_type" {
                        calls::extract_protocol_composition_refs(&type_node, src, source_symbol_index, refs);
                    } else {
                        calls::extract_type_ref_from_swift_type(&type_node, src, source_symbol_index, refs);
                    }
                }
            }
            "user_type" | "optional_type" | "array_type" | "dictionary_type"
            | "function_type" => {
                calls::extract_type_ref_from_swift_type(&child, src, source_symbol_index, refs);
            }
            "protocol_composition_type" => {
                calls::extract_protocol_composition_refs(&child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

