// =============================================================================
// parser/extractors/javascript.rs  —  JavaScript / JSX extractor
//
// Delegates entirely to the TypeScript extractor since the JavaScript CST
// produced by tree-sitter-javascript uses the same node kinds for all
// constructs we care about (class_declaration, function_declaration,
// method_definition, call_expression, import_statement, etc.).
//
// The TypeScript extractor already accepts `is_tsx = false` which selects the
// plain TypeScript grammar; here we feed it JavaScript source and use the JS
// grammar.  Because the JS grammar is a subset of the TS grammar at the node
// kind level, the walk logic in typescript.rs handles everything correctly.
// =============================================================================

use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::Parser;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract symbols and references from JavaScript (or JSX) source code.
///
/// Delegates the AST walk to the TypeScript extractor because JS and TS use
/// identical node kinds for all constructs we extract.  The only difference
/// is the grammar used to parse — we parse with `tree_sitter_javascript`
/// here before handing off the symbols/refs construction to the shared logic.
pub fn extract(source: &str) -> super::ExtractionResult {
    // Parse with the JavaScript grammar.
    let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load JavaScript grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return super::ExtractionResult::new(vec![], vec![], true);
        }
    };

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();

    // Re-use the TypeScript scope tree + walk logic.  We build the scope tree
    // with the JS tree (same root node structure) and then extract symbols
    // through the shared TypeScript walker.  Since both grammars emit the same
    // node kind strings for the constructs we handle, no translation is needed.
    use crate::parser::scope_tree::{self, ScopeKind};
    static JS_SCOPE_KINDS: &[ScopeKind] = &[
        ScopeKind { node_kind: "class_declaration",   name_field: "name" },
        ScopeKind { node_kind: "function_declaration", name_field: "name" },
    ];

    let root = tree.root_node();
    let scope_tree = scope_tree::build(root, src_bytes, JS_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_js_node(root, src_bytes, &scope_tree, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Node visitor — mirrors typescript::extract_node, JS node kinds only
// ---------------------------------------------------------------------------

use crate::parser::scope_tree::ScopeTree;
use crate::types::{EdgeKind, ExtractedRef as Ref, ExtractedSymbol as Sym, SymbolKind, Visibility};
use tree_sitter::Node;

fn extract_js_node(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    refs: &mut Vec<Ref>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                let idx = push_class(&child, src, scope_tree, symbols, parent_index);
                extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_js_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "function_declaration" => {
                let idx = push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    if let Some(sym_idx) = idx {
                        extract_calls(&body, src, sym_idx, refs);
                    }
                }
            }

            "export_statement" => {
                extract_js_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "method_definition" => {
                let idx = push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    if let Some(sym_idx) = idx {
                        extract_calls(&body, src, sym_idx, refs);
                    }
                }
            }

            "field_definition" => {
                push_field(&child, src, scope_tree, symbols, parent_index);
            }

            "lexical_declaration" | "variable_declaration" => {
                push_variable_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "import_statement" => {
                push_import(&child, src, symbols.len(), refs);
            }

            _ => {
                extract_js_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

fn push_class(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
) -> Option<usize> {
    use crate::parser::scope_tree;
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(Sym {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("class {name}")),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_function(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
) -> Option<usize> {
    use crate::parser::scope_tree;
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(Sym {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("function {name}{params}").trim().to_string()),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_method(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
) -> Option<usize> {
    use crate::parser::scope_tree;
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let kind = if name == "constructor" {
        SymbolKind::Constructor
    } else {
        SymbolKind::Method
    };

    let idx = symbols.len();
    symbols.push(Sym {
        name,
        qualified_name,
        kind,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_field(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;
    let name_node = match node.child_by_field_name("property") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    symbols.push(Sym {
        name,
        qualified_name,
        kind: SymbolKind::Property,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

fn push_variable_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if name_node.kind() == "identifier" {
                    let name = node_text(name_node, src);
                    let qualified_name = scope_tree::qualify(&name, parent_scope);
                    symbols.push(Sym {
                        name: name.clone(),
                        qualified_name,
                        kind: SymbolKind::Variable,
                        visibility: detect_visibility(node, src),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: Some(format!("const {name}")),
                        doc_comment: None,
                        scope_path: scope_path.clone(),
                        parent_index,
                    });
                }
            }
        }
    }
}

fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<Ref>,
) {
    let module_path = node.child_by_field_name("source").map(|s| {
        node_text(s, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            let mut ic = child.walk();
            for item in child.children(&mut ic) {
                match item.kind() {
                    "identifier" => {
                        refs.push(Ref {
                            source_symbol_index: current_symbol_count,
                            target_name: node_text(item, src),
                            kind: EdgeKind::TypeRef,
                            line: item.start_position().row as u32,
                            module: module_path.clone(),
                        });
                    }
                    "named_imports" => {
                        let mut ni = item.walk();
                        for spec in item.children(&mut ni) {
                            if spec.kind() == "import_specifier" {
                                let imported_name = spec
                                    .child_by_field_name("name")
                                    .map(|n| node_text(n, src))
                                    .unwrap_or_else(|| node_text(spec, src));
                                refs.push(Ref {
                                    source_symbol_index: current_symbol_count,
                                    target_name: imported_name,
                                    kind: EdgeKind::TypeRef,
                                    line: spec.start_position().row as u32,
                                    module: module_path.clone(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Heritage clause (extends)
// ---------------------------------------------------------------------------

fn extract_heritage(node: &Node, src: &[u8], source_idx: usize, refs: &mut Vec<Ref>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_heritage" {
            // JS grammar: class_heritage directly contains `extends` keyword
            // and then identifier(s), with no extends_clause wrapper.
            // Walk children: skip the `extends` keyword token, capture identifiers.
            let mut hc = child.walk();
            for n in child.children(&mut hc) {
                match n.kind() {
                    "identifier" => {
                        refs.push(Ref {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::Inherits,
                            line: n.start_position().row as u32,
                            module: None,
                        });
                    }
                    // TS-style extends_clause — handled if present
                    "extends_clause" => {
                        let mut ec = n.walk();
                        for type_node in n.children(&mut ec) {
                            if type_node.kind() == "identifier" {
                                refs.push(Ref {
                                    source_symbol_index: source_idx,
                                    target_name: node_text(type_node, src),
                                    kind: EdgeKind::Inherits,
                                    line: type_node.start_position().row as u32,
                                    module: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_calls(node: &Node, src: &[u8], source_symbol_index: usize, refs: &mut Vec<Ref>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let name = callee_name(func_node, src);
                if !name.is_empty() {
                    refs.push(Ref {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        module: None,
                    });
                }
            }
        }
        extract_calls(&child, src, source_symbol_index, refs);
    }
}

fn callee_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_expression" => {
            let obj = node
                .child_by_field_name("object")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            let prop = node
                .child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            if obj.is_empty() || prop.is_empty() {
                node_text(node, src)
            } else {
                format!("{obj}.{prop}")
            }
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export" {
            return Some(Visibility::Public);
        }
        let text = node_text(child, src);
        match text.as_str() {
            "public" => return Some(Visibility::Public),
            "private" => return Some(Visibility::Private),
            _ => {}
        }
    }
    None
}

fn extract_jsdoc(node: &Node, src: &[u8]) -> Option<String> {
    let sib = node.prev_sibling()?;
    if sib.kind() == "comment" {
        let text = node_text(sib, src);
        if text.starts_with("/**") {
            return Some(text);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "javascript_tests.rs"]
mod tests;
