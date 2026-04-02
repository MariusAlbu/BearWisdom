// =============================================================================
// rust/symbols.rs  —  Symbol extractors for the Rust extractor
// =============================================================================

use super::helpers::{
    detect_visibility, extract_doc_comment, extract_signature, node_text, qualify, scope_from_prefix,
};
use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn extract_function(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_signature(node, source);

    let kind = if super::helpers::has_test_attribute(node, source) {
        SymbolKind::Test
    } else {
        SymbolKind::Function
    };

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// Same as `extract_function` but always emits `Method` kind (used inside impl blocks).
pub(super) fn extract_method_from_fn(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_signature(node, source);

    let kind = if super::helpers::has_test_attribute(node, source) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_struct(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("struct {name}");
    if let Some(tp) = node.child_by_field_name("type_parameters") {
        sig.push_str(&node_text(&tp, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Struct,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_enum(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("enum {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Enum,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// Extract `enum_variant` children from an enum body into the symbol list.
pub(super) fn extract_enum_variants(
    body: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            // tree-sitter-rust uses `name` field on enum_variant nodes.
            // Fall back to the first named identifier child if the field is missing.
            let field_name_node = child.child_by_field_name("name");
            let name_node = if field_name_node.is_some() {
                field_name_node
            } else {
                let mut variant_cursor = child.walk();
                let found = child
                    .children(&mut variant_cursor)
                    .find(|n| n.is_named() && n.kind() == "identifier");
                found
            };

            if let Some(name_node) = name_node {
                let name = node_text(&name_node, source);
                let qualified_name = qualify(&name, qualified_prefix);
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::EnumMember,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: extract_doc_comment(&child, source),
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
    }
}

pub(super) fn extract_trait(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("trait {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Interface,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_type_alias(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("type {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_const(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("const {name}");
    if let Some(ty) = node.child_by_field_name("type") {
        sig.push_str(": ");
        sig.push_str(&node_text(&ty, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_static(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("static {name}");
    if let Some(ty) = node.child_by_field_name("type") {
        sig.push_str(": ");
        sig.push_str(&node_text(&ty, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// `macro_rules! foo { ... }` — emit a Function symbol for the macro name.
///
/// tree-sitter-rust 0.24 shape (`macro_definition` node):
/// ```text
/// macro_definition
///   name: identifier  "foo"
///   macro_rule+
/// ```
pub(super) fn extract_macro_rules(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    if name.is_empty() {
        return None;
    }
    let qualified_name = qualify(&name, qualified_prefix);
    let doc_comment = extract_doc_comment(node, source);

    Some(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: detect_visibility(node),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("macro_rules! {name}")),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_mod(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("mod {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}
