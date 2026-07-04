// =============================================================================
// languages/typescript/annotation_members.rs — object-type members of a
// variable declarator's type annotation
//
// `declare var AbortSignal: { any(signals): AbortSignal }` declares members
// on an object type whose enclosing declaration is a variable, not a
// class/interface/namespace. The scope tree registers none of these object
// types as scopes, so a plain extraction walk would emit the members with
// bare qualified names (`any`) and no parent — and a bare method qname
// collides with every same-named lookup in the index. The walk here parents
// each member under the declared variable's symbol and derives its qualified
// name from that parent chain (`AbortSignal.any`).
// =============================================================================

use super::extract::recurse_for_object_types;
use super::helpers::node_text;
use crate::types::{AliasTarget, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

/// Emit the members of every object type found in the type annotations of an
/// identifier-named declarator of `decl` (a `lexical_declaration` /
/// `variable_declaration`), parented under the declarator's Variable/Function
/// symbol and qualified by the parent chain.
///
/// `decl_syms_start` is `symbols.len()` from just before `push_variable_decl`
/// ran for `decl` — the declarator symbols to parent under are found in
/// `symbols[decl_syms_start..]`.
pub(super) fn push_annotation_object_members(
    decl: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    alias_targets: &mut Vec<(String, AliasTarget)>,
    decl_syms_start: usize,
) {
    let mut cursor = decl.walk();
    let declarators: Vec<Node> = decl
        .children(&mut cursor)
        .filter(|c| c.kind() == "variable_declarator")
        .collect();
    drop(cursor);

    for declarator in declarators {
        let Some(name_node) = declarator.child_by_field_name("name") else {
            continue;
        };
        if name_node.kind() != "identifier" {
            continue;
        }
        let Some(type_ann) = declarator.child_by_field_name("type") else {
            continue;
        };
        let name = node_text(name_node, src);
        let Some(var_idx) = symbols[decl_syms_start..]
            .iter()
            .position(|s| {
                s.name == name && matches!(s.kind, SymbolKind::Variable | SymbolKind::Function)
            })
            .map(|p| p + decl_syms_start)
        else {
            continue;
        };
        // type_annotation ::= ":" type — skip the ":" token.
        let mut tc = type_ann.walk();
        let type_value = type_ann.children(&mut tc).find(|c| c.kind() != ":");
        drop(tc);
        let Some(tv) = type_value else {
            continue;
        };

        let walk_start = symbols.len();
        recurse_for_object_types(
            tv,
            src,
            scope_tree,
            symbols,
            refs,
            alias_targets,
            Some(var_idx),
        );
        // The walk stamps scope-tree qnames, which never see object types.
        // Re-derive each member's qname from its parent chain — parents are
        // pushed before their children, so a forward pass reads already
        // re-derived parent qnames and nested members come out fully dotted
        // (`X.a.b`, not a flattened `X.b`).
        for i in walk_start..symbols.len() {
            let Some(p) = symbols[i].parent_index else {
                continue;
            };
            let qname = format!("{}.{}", symbols[p].qualified_name, symbols[i].name);
            symbols[i].qualified_name = qname;
        }
    }
}

/// True when `annotation` is the `type` field of an identifier-named
/// `variable_declarator` — the shape whose object-type members
/// `push_annotation_object_members` emits. The generic extraction walk must
/// not descend into such an annotation, or the members would be re-emitted
/// with bare names.
pub(super) fn is_declarator_annotation(parent: &Node, annotation: &Node) -> bool {
    parent.kind() == "variable_declarator"
        && parent
            .child_by_field_name("name")
            .is_some_and(|n| n.kind() == "identifier")
        && parent
            .child_by_field_name("type")
            .is_some_and(|t| t.id() == annotation.id())
}
