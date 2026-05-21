// =============================================================================
// languages/typescript/selectors.rs — Angular selector extraction
//
// Recognizes two source forms:
//   * `@Component({selector:'...'})` / `@Directive({selector:'...'})`
//     decorators on class declarations — the project-source form.
//   * `X.ɵdir = i0.ɵɵngDeclareDirective({selector:"...", type: X, ...})`
//     and the matching `ɵɵngDeclareComponent` — the Angular Ivy compiled
//     metadata form emitted into `node_modules/<pkg>/fesm*/...mjs`. Without
//     this path, every reference to a third-party Angular component (CoreUI's
//     `<c-container>`, Material's `mat-*`, RouterModule's `<router-outlet>`)
//     fell through to unresolved.
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

#[cfg(test)]
#[path = "selectors_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Angular @Component selector extraction (called by full-index pipeline)
// ---------------------------------------------------------------------------

/// Scan `source` for `@Component({selector: '...'})` decorators on class
/// declarations and return a mapping of `(raw_selector, class_qualified_name)`
/// pairs.
///
/// `symbols` must be the symbols already extracted from the same source (via
/// `extract` or `extract_with_demand`) — this function matches the N-th class
/// declaration in the AST to the N-th `SymbolKind::Class` symbol in the
/// vec to obtain the qualified name without re-running the full symbol
/// extraction logic.
///
/// Called by the full-index pipeline (`indexer/full.rs`) for `typescript` and
/// `angular` files so `SymbolIndex::build_with_context` can build the
/// project-wide Angular selector map without a second parse pass per file.
pub fn extract_component_selectors(
    source: &str,
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<(String, String)> {
    use crate::types::SymbolKind;

    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let src = source.as_bytes();
    let root = tree.root_node();

    // Pre-build an index of class symbol qualified names in declaration order.
    // When a class declaration with an @Component decorator is encountered during
    // the AST walk, we match it by its class-name node text against the symbols
    // vec to get its qualified_name.
    let class_qnames: std::collections::HashMap<String, String> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .map(|s| (s.name.clone(), s.qualified_name.clone()))
        .collect();

    let mut result: Vec<(String, String)> = Vec::new();
    collect_component_selectors_recursive(&root, src, &class_qnames, &mut result);
    result
}

fn collect_component_selectors_recursive(
    node: &tree_sitter::Node,
    src: &[u8],
    class_qnames: &std::collections::HashMap<String, String>,
    result: &mut Vec<(String, String)>,
) {
    let kind = node.kind();
    if matches!(kind, "class_declaration" | "abstract_class_declaration") {
        // Try to extract a selector from a @Component decorator on this class.
        let selectors = super::decorators::component_selectors_from_class(node, src);
        if !selectors.is_empty() {
            // Get the class name to look up its qualified name.
            if let Some(name_node) = node.child_by_field_name("name") {
                let class_name = super::helpers::node_text(name_node, src);
                if let Some(qname) = class_qnames.get(&class_name) {
                    for sel in selectors {
                        result.push((sel, qname.clone()));
                    }
                }
            }
        }
        // .d.ts form: scan class body for `static ɵdir: ɵɵDirectiveDeclaration<X, "selector", ...>`
        // and the matching `ɵcmp: ɵɵComponentDeclaration<...>`.
        try_extract_ng_declaration_field(node, src, class_qnames, result);
    }
    if kind == "call_expression" {
        try_extract_ng_declare_call(node, src, class_qnames, result);
    }
    // Recurse into children to handle nested classes and export wrappers.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_component_selectors_recursive(&child, src, class_qnames, result);
    }
}

/// Match the `.d.ts` Angular Ivy metadata pattern emitted by
/// `tsc --emitDeclarationOnly` on Angular libraries:
///
/// ```text
/// class C {
///     static ɵdir: _angular_core.ɵɵDirectiveDeclaration<C, "[cFoo]", ...>;
///     static ɵcmp: _angular_core.ɵɵComponentDeclaration<C, "c-foo", ...>;
/// }
/// ```
///
/// The selector is the 2nd type-argument string literal. The class is the
/// enclosing `class_declaration` (passed as `class_node`).
fn try_extract_ng_declaration_field(
    class_node: &tree_sitter::Node,
    src: &[u8],
    class_qnames: &std::collections::HashMap<String, String>,
    result: &mut Vec<(String, String)>,
) {
    let Some(name_node) = class_node.child_by_field_name("name") else { return };
    let class_name = super::helpers::node_text(name_node, src);
    let Some(qname) = class_qnames.get(&class_name) else { return };

    let Some(body) = class_node.child_by_field_name("body") else { return };
    let mut cursor = body.walk();
    for member in body.children(&mut cursor) {
        // Both `public_field_definition` (concrete class) and
        // `property_signature` (declare class / interface) carry the
        // `static ɵdir: …` shape we're scanning for.
        if !matches!(member.kind(), "public_field_definition" | "property_signature") {
            continue;
        }
        let Some(type_anno) = field_type_annotation(&member) else { continue };
        let Some(generic) = first_generic_type_in_anno(&type_anno) else { continue };
        let Some(base_name) = generic_base_tail(&generic, src) else { continue };
        if base_name != "ɵɵDirectiveDeclaration" && base_name != "ɵɵComponentDeclaration" {
            continue;
        }
        let Some(type_args) = generic.child_by_field_name("type_arguments") else { continue };
        let Some(selector_raw) = nth_type_arg_string(&type_args, 1, src) else { continue };
        for sel in super::decorators::split_and_normalize_selectors(&selector_raw) {
            result.push((sel, qname.clone()));
        }
    }
}

/// Pull the named `type` child of a class member node (the type annotation
/// without the leading colon token).
fn field_type_annotation<'a>(member: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    member.child_by_field_name("type")
}

/// First `generic_type` node inside a `type_annotation` wrapper. The
/// `type_annotation` node has the form `(type_annotation (generic_type ...))`
/// so we walk its named children for the generic.
fn first_generic_type_in_anno<'a>(anno: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = anno.walk();
    for child in anno.children(&mut cursor) {
        if child.kind() == "generic_type" {
            return Some(child);
        }
    }
    None
}

/// Return the trailing identifier of a `generic_type`'s base — for
/// `_angular_core.ɵɵDirectiveDeclaration<…>` returns
/// `"ɵɵDirectiveDeclaration"`. Handles both `type_identifier` (bare) and
/// `nested_type_identifier` (`module.Name`) shapes by reading the `name`
/// field on the latter.
fn generic_base_tail(generic: &tree_sitter::Node, src: &[u8]) -> Option<String> {
    let base = generic.child_by_field_name("name")?;
    if base.kind() == "nested_type_identifier" {
        let last = base.child_by_field_name("name")?;
        return Some(super::helpers::node_text(last, src));
    }
    Some(super::helpers::node_text(base, src))
}

/// Pull the N-th positional type argument from a `type_arguments` node,
/// unquoting it if it's a string-literal type. Returns `None` for indices
/// outside the list or when the argument isn't a literal string.
fn nth_type_arg_string(args: &tree_sitter::Node, n: usize, src: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    let mut idx = 0;
    for child in args.children(&mut cursor) {
        if !child.is_named() { continue; }
        if idx == n {
            // `literal_type` wraps the string literal in declaration types.
            if child.kind() == "literal_type" {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if matches!(inner.kind(), "string" | "template_string") {
                        return unquote(&inner, src);
                    }
                }
                return None;
            }
            if matches!(child.kind(), "string" | "template_string") {
                return unquote(&child, src);
            }
            return None;
        }
        idx += 1;
    }
    None
}

/// Match `<ns>.ɵɵngDeclareDirective({...})` and
/// `<ns>.ɵɵngDeclareComponent({...})` calls. Extracts the `selector` and
/// `type` fields from the object argument and emits a (selector, class_qname)
/// pair when both are present and the class is in `class_qnames`.
fn try_extract_ng_declare_call(
    call: &tree_sitter::Node,
    src: &[u8],
    class_qnames: &std::collections::HashMap<String, String>,
    result: &mut Vec<(String, String)>,
) {
    let Some(func) = call.child_by_field_name("function") else { return };
    if func.kind() != "member_expression" {
        return;
    }
    let Some(prop) = func.child_by_field_name("property") else { return };
    let prop_text = super::helpers::node_text(prop, src);
    if prop_text != "ɵɵngDeclareDirective" && prop_text != "ɵɵngDeclareComponent" {
        return;
    }
    let Some(args) = call.child_by_field_name("arguments") else { return };
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        if arg.kind() != "object" {
            continue;
        }
        let mut selector_values: Vec<String> = Vec::new();
        let mut type_name: Option<String> = None;
        let mut oc = arg.walk();
        for prop in arg.children(&mut oc) {
            if prop.kind() != "pair" { continue; }
            let Some(key) = prop.child_by_field_name("key") else { continue };
            let key_text = super::helpers::node_text(key, src);
            let Some(val) = prop.child_by_field_name("value") else { continue };
            match key_text.as_str() {
                "selector" => {
                    if let Some(raw) = unquote(&val, src) {
                        selector_values = super::decorators::split_and_normalize_selectors(&raw);
                    }
                }
                "type" => {
                    // `type: ClassName` is an identifier; that's the class
                    // this metadata describes.
                    if val.kind() == "identifier" {
                        type_name = Some(super::helpers::node_text(val, src));
                    }
                }
                _ => {}
            }
        }
        if let (Some(name), false) = (type_name, selector_values.is_empty()) {
            if let Some(qname) = class_qnames.get(&name) {
                for sel in selector_values {
                    result.push((sel, qname.clone()));
                }
            }
        }
    }
}

fn unquote(node: &tree_sitter::Node, src: &[u8]) -> Option<String> {
    let raw = super::helpers::node_text(*node, src);
    if raw.is_empty() { return None; }
    let stripped = raw
        .trim_start_matches('`')
        .trim_end_matches('`')
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string();
    if stripped.is_empty() { None } else { Some(stripped) }
}

