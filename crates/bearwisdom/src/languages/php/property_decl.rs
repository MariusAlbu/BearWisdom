// =============================================================================
// php/property_decl.rs  —  class property declarations and their type evidence
// =============================================================================

use super::calls::extract_type_refs_from_php_type;
use super::helpers::{adjacent_phpdoc, extract_visibility, node_text, qualify, scope_from_prefix};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

/// Extract every property a `property_declaration` declares, plus the one
/// piece of type evidence the declaration carries: its native hint when it
/// has one, otherwise the class its docblock's `@var` tag names.
pub(super) fn extract_property_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let visibility = extract_visibility(node, src);
    let phpdoc = adjacent_phpdoc(node, src);
    let doc_type = phpdoc.as_deref().and_then(doc_var_type);

    // The property type hint is a direct child of the property_declaration node.
    // (Not inside property_element — it's a sibling of property_element.)
    let type_node_opt: Option<Node> = {
        let mut cc = node.walk();
        let mut found = None;
        for child in node.children(&mut cc) {
            match child.kind() {
                "named_type"
                | "nullable_type"
                | "union_type"
                | "intersection_type"
                | "disjunctive_normal_form_type" => {
                    found = Some(child);
                    break;
                }
                _ => {}
            }
        }
        found
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "property_element" {
            let mut vc = child.walk();
            for var in child.children(&mut vc) {
                if var.kind() == "variable_name" || var.kind() == "$variable_name" {
                    let raw = node_text(&var, src);
                    let name = raw.trim_start_matches('$').to_string();
                    let qualified_name = qualify(&name, qualified_prefix);
                    let prop_idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        name,
                        qualified_name,
                        kind: SymbolKind::Property,
                        visibility,
                        start_line: var.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: var.start_position().column as u32,
                        end_col: node.end_position().column as u32,
                        signature: None,
                        doc_comment: phpdoc.clone(),
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                        byte_offset: 0,
                        declared_type: None,
                        return_type: None,
                        param_types: Vec::new(),
                        generic_params: Vec::new(),
                    });
                    emit_property_type(
                        type_node_opt,
                        doc_type.as_deref(),
                        &var,
                        src,
                        refs,
                        prop_idx,
                    );
                    break;
                }
            }
        }
    }
}

/// The declaration's own type evidence for one property. A native hint is the
/// declaration's contract and suppresses the docblock, so exactly one TypeRef
/// reaches the property and the first one is the native hint wherever one exists.
fn emit_property_type(
    type_node: Option<Node>,
    doc_type: Option<&str>,
    var: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    prop_idx: usize,
) {
    match type_node {
        Some(tn) => extract_type_refs_from_php_type(&tn, src, refs, prop_idx),
        None => {
            if let Some(written) = doc_type {
                super::type_ref_emit::emit_php_type_ref(
                    written,
                    var.start_position().row as u32,
                    var.start_byte() as u32,
                    refs,
                    prop_idx,
                );
            }
        }
    }
}

/// The single class a property's `@var` tag names, spelled as the tag writes
/// it. Abstains on every form whose class identity is not unambiguous —
/// array/generic/shape brackets, a multi-class union, a scalar or pseudo type.
/// `?T` and `T|null` collapse to `T`. The scalar/pseudo rejection belongs to
/// the emitter, which owns the grammar's own type spellings.
fn doc_var_type(phpdoc: &str) -> Option<String> {
    // The block delimiters go first so a one-line `/** @var T */` reads the
    // same as a tag on its own `*`-prefixed line. Last tag wins, matching
    // `adjacent_phpdoc`'s own reading of the block.
    let body = phpdoc
        .trim()
        .trim_start_matches("/**")
        .trim_end_matches("*/");
    let token = body
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches('*').trim_start();
            let rest = line.strip_prefix("@var")?;
            rest.strip_prefix(char::is_whitespace)?
                .split_whitespace()
                .next()
        })
        .next_back()?;
    if token.contains(['[', ']', '<', '>', '{', '}', '(', ')', ',']) {
        return None;
    }
    let mut branches = token
        .strip_prefix('?')
        .unwrap_or(token)
        .split('|')
        .filter(|branch| !branch.is_empty() && !branch.eq_ignore_ascii_case("null"));
    let only = branches.next()?;
    branches.next().is_none().then(|| only.to_string())
}
