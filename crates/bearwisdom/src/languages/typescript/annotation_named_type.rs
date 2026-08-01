// =============================================================================
// languages/typescript/annotation_named_type.rs — named-type annotation of an
// ambient variable declarator
//
// `declare const api: ApiType` binds a value whose whole member surface lives
// on the named annotation type. The declarator symbol's signature is emitted
// as `const api` (annotation-less), so the arena pass that fills
// `declared_type` from `name: Type` signatures has nothing to parse, and the
// annotation's TypeRef is rewritten to the import's source name by the
// import-semantics pass — after which no ref carries the local type name the
// declaration was annotated with. Enriching the signature with the annotation
// text (`const api: ApiType`) lets the existing signature-driven fill capture
// the declared type at the symbol.
//
// Ambient declarators only: a non-ambient `const x: T` initializer is typed by
// the flow seed and the scope-qualified ref derivation, which the extractor-set
// TypeId would out-rank with an unqualified name.
// =============================================================================

use super::helpers::node_text;
use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

/// Rewrite the signature of each identifier-named declarator of `decl` (an
/// ambient `lexical_declaration` / `variable_declaration`) whose type
/// annotation is a named type, from `const {name}` to `const {name}: {type}`.
///
/// `decl_syms_start` is `symbols.len()` from just before `push_variable_decl`
/// ran for `decl` — the declarator symbols to enrich are found in
/// `symbols[decl_syms_start..]`.
pub(super) fn enrich_ambient_declarator_signatures(
    decl: &Node,
    src: &[u8],
    symbols: &mut [ExtractedSymbol],
    decl_syms_start: usize,
) {
    // `declare const x: T`, and the same declaration without the keyword — in a
    // declaration file every top-level binding is ambient, and the grammar
    // marks only the keyword form. The structural test is the absence of an
    // INITIALIZER: `const x: T` with no `= …` cannot be typed by a flow seed,
    // so recording its annotation can never out-rank a seeded type.
    let ambient_keyword = decl
        .parent()
        .is_some_and(|p| p.kind() == "ambient_declaration");
    if !ambient_keyword && !declarators_are_initializerless(decl) {
        return;
    }
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
        // type_annotation ::= ":" type — skip the ":" token.
        let mut tc = type_ann.walk();
        let type_value = type_ann.children(&mut tc).find(|c| c.kind() != ":");
        drop(tc);
        let Some(tv) = type_value else {
            continue;
        };
        // Named shapes, plus an INTERSECTION of them: `const v: A & B` carries
        // every arm's members, and `intern_type_str` decomposes the recorded
        // annotation into the structural intersection the member walk traverses
        // arm by arm. A union is deliberately excluded — its member set is the
        // arms' INTERSECTION, which the walker derives from the arms
        // themselves, so recording one here would not widen anything.
        // An object-type annotation's members are emitted as symbols by
        // `annotation_members`; a function/constructor type has no nominal
        // member surface to root on.
        if !matches!(
            tv.kind(),
            "type_identifier" | "nested_type_identifier" | "generic_type" | "intersection_type"
        ) {
            continue;
        }
        let name = node_text(name_node, src);
        let ty = node_text(tv, src);
        if ty.is_empty() {
            continue;
        }
        let Some(sym) = symbols[decl_syms_start..].iter_mut().find(|s| {
            s.name == name && matches!(s.kind, SymbolKind::Variable | SymbolKind::Function)
        }) else {
            continue;
        };
        sym.signature = Some(format!("const {name}: {ty}"));
    }
}

/// True when every declarator of `decl` carries a type annotation and no
/// initializer — the shape a declaration file uses for an ambient binding.
fn declarators_are_initializerless(decl: &Node) -> bool {
    let mut cursor = decl.walk();
    let declarators: Vec<Node> = decl
        .children(&mut cursor)
        .filter(|c| c.kind() == "variable_declarator")
        .collect();
    drop(cursor);
    !declarators.is_empty()
        && declarators.iter().all(|d| {
            d.child_by_field_name("type").is_some() && d.child_by_field_name("value").is_none()
        })
}

#[cfg(test)]
#[path = "annotation_named_type_tests.rs"]
mod tests;
