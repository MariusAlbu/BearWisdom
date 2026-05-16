// =============================================================================
// languages/typescript/selectors.rs — Angular @Component selector extraction
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

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
    }
    // Recurse into children to handle nested classes and export wrappers.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_component_selectors_recursive(&child, src, class_qnames, result);
    }
}

