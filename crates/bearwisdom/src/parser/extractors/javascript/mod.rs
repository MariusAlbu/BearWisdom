// =============================================================================
// parser/extractors/javascript/mod.rs  —  JavaScript / JSX extractor
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

mod helpers;

use helpers::{detect_visibility, extract_jsdoc, node_text};

use crate::parser::scope_tree::ScopeTree;
use crate::types::{EdgeKind, ExtractedRef as Ref, ExtractedSymbol as Sym, SymbolKind};
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract symbols and references from JavaScript (or JSX) source code.
pub fn extract(source: &str) -> super::ExtractionResult {
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

    use crate::parser::scope_tree::{self, ScopeKind};

    static JS_SCOPE_KINDS: &[ScopeKind] = &[
        ScopeKind { node_kind: "class_declaration", name_field: "name" },
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
// Node visitor
// ---------------------------------------------------------------------------

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

            "function_declaration" | "generator_function_declaration" => {
                let idx = push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    if let Some(sym_idx) = idx {
                        extract_calls(&body, src, sym_idx, refs);
                        // Recurse for nested declarations inside function bodies.
                        extract_js_node(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }

            "export_statement" => {
                // `export class Foo {}` / `export function bar() {}` / `export default class {}`
                // Recurse so the child declaration nodes hit their own arms.
                extract_js_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "method_definition" => {
                let idx = push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    if let Some(sym_idx) = idx {
                        extract_calls(&body, src, sym_idx, refs);
                        extract_js_node(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }

            "field_definition" => {
                push_field(&child, src, scope_tree, symbols, parent_index);
            }

            "lexical_declaration" | "variable_declaration" => {
                push_variable_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "import_statement" => {
                push_import(&child, src, symbols.len(), refs);
            }

            // `for (const item of items)` / `for (const key in obj)` —
            // tree-sitter uses `for_in_statement` for both `for...in` and `for...of`.
            "for_in_statement" => {
                extract_for_loop_var(&child, src, scope_tree, symbols, refs, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_js_node(body, src, scope_tree, symbols, refs, parent_index);
                }
            }

            // `catch (e) { ... }` — extract the binding as a Variable symbol.
            "catch_clause" => {
                extract_catch_variable(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_js_node(body, src, scope_tree, symbols, refs, parent_index);
                }
            }

            // `expression_statement` may wrap `module.exports = ...` or
            // `exports.X = ...` assignments.
            "expression_statement" => {
                extract_module_exports(&child, src, symbols.len(), refs);
                extract_js_node(child, src, scope_tree, symbols, refs, parent_index);
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

/// Push variable declarators from `const`/`let`/`var` declarations.
///
/// Handles:
/// - Simple identifiers: `const foo = ...` → Variable symbol
/// - `arrow_function` initializer: `const foo = (x) => x` → Function symbol
/// - `function_expression` initializer: `const foo = function() {}` → Function symbol
/// - `class` expression initializer: `const Foo = class { }` → Class symbol (name from variable; JS grammar uses "class" node kind)
/// - `generator_function` initializer: `const gen = function* () {}` → Function symbol
/// - Object destructuring: `const { a, b } = obj` → one Variable per binding
/// - Rest in destructuring: `const { a, ...rest } = obj` → Variable for `rest`
fn push_variable_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    refs: &mut Vec<Ref>,
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
        if child.kind() != "variable_declarator" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };

        match name_node.kind() {
            "identifier" => {
                let name = node_text(name_node, src);
                let qualified_name = scope_tree::qualify(&name, parent_scope);

                // Inspect the initializer to pick the right symbol kind.
                let init = child.child_by_field_name("value");
                let init_kind = init.as_ref().map(|n| n.kind()).unwrap_or("");

                match init_kind {
                    // `const foo = (x) => x + 1` — emit as Function
                    "arrow_function" => {
                        let params = init
                            .as_ref()
                            .and_then(|n| n.child_by_field_name("parameters").or_else(|| {
                                // Single-param shorthand: `x => x` — the param is the
                                // `identifier` child with field name "parameter".
                                n.child_by_field_name("parameter")
                            }))
                            .map(|p| node_text(p, src))
                            .unwrap_or_default();
                        let idx = symbols.len();
                        symbols.push(Sym {
                            name: name.clone(),
                            qualified_name,
                            kind: SymbolKind::Function,
                            visibility: detect_visibility(node, src),
                            start_line: child.start_position().row as u32,
                            end_line: child.end_position().row as u32,
                            start_col: child.start_position().column as u32,
                            end_col: child.end_position().column as u32,
                            signature: Some(format!("const {name} = ({params}) =>")),
                            doc_comment: extract_jsdoc(node, src),
                            scope_path: scope_path.clone(),
                            parent_index,
                        });
                        // Extract calls from arrow body.
                        if let Some(init_node) = &init {
                            if let Some(body) = init_node.child_by_field_name("body") {
                                extract_calls(&body, src, idx, refs);
                            }
                        }
                    }

                    // `const foo = function bar() {}` / `const gen = function* () {}`
                    "function_expression" | "generator_function" => {
                        let params = init
                            .as_ref()
                            .and_then(|n| n.child_by_field_name("parameters"))
                            .map(|p| node_text(p, src))
                            .unwrap_or_default();
                        let idx = symbols.len();
                        symbols.push(Sym {
                            name: name.clone(),
                            qualified_name,
                            kind: SymbolKind::Function,
                            visibility: detect_visibility(node, src),
                            start_line: child.start_position().row as u32,
                            end_line: child.end_position().row as u32,
                            start_col: child.start_position().column as u32,
                            end_col: child.end_position().column as u32,
                            signature: Some(format!("function {name}{params}")),
                            doc_comment: extract_jsdoc(node, src),
                            scope_path: scope_path.clone(),
                            parent_index,
                        });
                        if let Some(init_node) = &init {
                            if let Some(body) = init_node.child_by_field_name("body") {
                                extract_calls(&body, src, idx, refs);
                            }
                        }
                    }

                    // `const Foo = class { ... }` — variable name is the class name.
                    // tree-sitter-javascript uses "class" for class expressions (not "class_expression").
                    "class" => {
                        let idx = symbols.len();
                        symbols.push(Sym {
                            name: name.clone(),
                            qualified_name,
                            kind: SymbolKind::Class,
                            visibility: detect_visibility(node, src),
                            start_line: child.start_position().row as u32,
                            end_line: child.end_position().row as u32,
                            start_col: child.start_position().column as u32,
                            end_col: child.end_position().column as u32,
                            signature: Some(format!("class {name}")),
                            doc_comment: extract_jsdoc(node, src),
                            scope_path: scope_path.clone(),
                            parent_index,
                        });
                        // Recurse into the class body for methods/fields.
                        if let Some(init_node) = &init {
                            if let Some(body) = init_node.child_by_field_name("body") {
                                extract_js_node_inner(body, src, scope_tree, symbols, refs, Some(idx));
                            }
                        }
                    }

                    // Everything else — plain Variable symbol.
                    _ => {
                        let idx = symbols.len();
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
                        // require('foo') → Imports edge
                        if let Some(init_node) = &init {
                            try_emit_require(init_node, src, idx, refs);
                        }
                    }
                }
            }

            // `const { a, b, ...rest } = obj` — object destructuring pattern.
            "object_pattern" => {
                let mut ppcursor = name_node.walk();
                for prop in name_node.children(&mut ppcursor) {
                    match prop.kind() {
                        // `{ a }` shorthand
                        "shorthand_property_identifier_pattern" => {
                            let prop_name = node_text(prop, src);
                            if !prop_name.is_empty() {
                                push_destructured_var(
                                    &prop_name, &prop, src, scope_tree, symbols, parent_index,
                                    &scope_path,
                                );
                            }
                        }
                        // `{ key: localName }` — use the value (localName)
                        "pair_pattern" => {
                            if let Some(val) = prop.child_by_field_name("value") {
                                if val.kind() == "identifier" {
                                    let prop_name = node_text(val, src);
                                    if !prop_name.is_empty() {
                                        push_destructured_var(
                                            &prop_name, &val, src, scope_tree, symbols,
                                            parent_index, &scope_path,
                                        );
                                    }
                                }
                            }
                        }
                        // `{ ...rest }` — rest element
                        "rest_pattern" => {
                            // The identifier inside the rest_pattern.
                            let mut rc = prop.walk();
                            for rest_child in prop.children(&mut rc) {
                                if rest_child.kind() == "identifier" {
                                    let rest_name = node_text(rest_child, src);
                                    if !rest_name.is_empty() {
                                        push_destructured_var(
                                            &rest_name, &rest_child, src, scope_tree, symbols,
                                            parent_index, &scope_path,
                                        );
                                    }
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // `const [a, b] = arr` — array destructuring.
            "array_pattern" => {
                let mut ac = name_node.walk();
                for elem in name_node.children(&mut ac) {
                    if elem.kind() == "identifier" {
                        let elem_name = node_text(elem, src);
                        if !elem_name.is_empty() {
                            push_destructured_var(
                                &elem_name, &elem, src, scope_tree, symbols, parent_index,
                                &scope_path,
                            );
                        }
                    } else if elem.kind() == "rest_pattern" {
                        let mut rc = elem.walk();
                        for rest_child in elem.children(&mut rc) {
                            if rest_child.kind() == "identifier" {
                                let rest_name = node_text(rest_child, src);
                                if !rest_name.is_empty() {
                                    push_destructured_var(
                                        &rest_name, &rest_child, src, scope_tree, symbols,
                                        parent_index, &scope_path,
                                    );
                                }
                                break;
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

/// Emit a single Variable symbol for a destructured binding.
fn push_destructured_var(
    name: &str,
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
    scope_path: &Option<String>,
) {
    use crate::parser::scope_tree;
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(name, parent_scope);
    symbols.push(Sym {
        name: name.to_string(),
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_path.clone(),
        parent_index,
    });
}

/// Inner recursion for class bodies (method_definition, field_definition only).
/// Avoids re-running top-level logic inside class bodies recursed from
/// `class_expression` initializers.
fn extract_js_node_inner(
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
            "method_definition" => {
                let idx = push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    if let Some(sym_idx) = idx {
                        extract_calls(&body, src, sym_idx, refs);
                        extract_js_node_inner(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }
            "field_definition" => {
                push_field(&child, src, scope_tree, symbols, parent_index);
            }
            _ => {
                extract_js_node_inner(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

fn push_import(node: &Node, src: &[u8], current_symbol_count: usize, refs: &mut Vec<Ref>) {
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
                            chain: None,
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
                                    chain: None,
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
                            chain: None,
                        });
                    }
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
                                    chain: None,
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
        match child.kind() {
            "call_expression" => {
                if let Some(func_node) = child.child_by_field_name("function") {
                    let callee = callee_name(func_node, src);

                    // require('foo') → Imports edge instead of Calls
                    if callee == "require" {
                        if let Some(module) = extract_require_path(&child, src) {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: module.clone(),
                                kind: EdgeKind::Imports,
                                line: child.start_position().row as u32,
                                module: Some(module),
                                chain: None,
                            });
                        }
                    }
                    // import('foo') — dynamic import → Imports edge
                    else if callee == "import" {
                        if let Some(module) = extract_first_string_arg(&child, src) {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: module.clone(),
                                kind: EdgeKind::Imports,
                                line: child.start_position().row as u32,
                                module: Some(module),
                                chain: None,
                            });
                        }
                    }
                    // Regular call
                    else if !callee.is_empty() {
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: callee,
                            kind: EdgeKind::Calls,
                            line: func_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            // `new Foo(args)` → Calls edge to the constructor
            "new_expression" => {
                if let Some(constructor) = child.child_by_field_name("constructor") {
                    let name = callee_name(constructor, src);
                    if !name.is_empty() {
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: constructor.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            // `` html`<div>` `` / `` gql`query {}` `` — tag is the called function.
            "tagged_template_expression" => {
                if let Some(tag) = child.child_by_field_name("tag") {
                    let name = callee_name(tag, src);
                    if !name.is_empty() {
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: tag.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls(&child, src, source_symbol_index, refs);
            }
        }
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

/// Extract the string argument from `require('foo')`.
/// Returns `None` if the call has no string literal argument.
fn extract_require_path(call_node: &Node, src: &[u8]) -> Option<String> {
    extract_first_string_arg(call_node, src)
}

/// Extract the first string literal from a call's arguments node.
fn extract_first_string_arg(call_node: &Node, src: &[u8]) -> Option<String> {
    let args = call_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        match arg.kind() {
            "string" | "template_string" => {
                let raw = node_text(arg, src);
                let cleaned = raw
                    .trim_matches('`')
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                return Some(cleaned);
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// module.exports / exports.X detection
// ---------------------------------------------------------------------------

/// Walk the expression_statement for `module.exports = X` or `exports.Foo = X`
/// assignments and emit an Imports edge pointing to the assigned name, with
/// the target set to the RHS identifier (if simple) so the indexer can link
/// the export to the source symbol.
fn extract_module_exports(
    stmt_node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<Ref>,
) {
    let mut cursor = stmt_node.walk();
    for child in stmt_node.children(&mut cursor) {
        if child.kind() != "assignment_expression" {
            continue;
        }
        let Some(left) = child.child_by_field_name("left") else {
            continue;
        };
        let Some(right) = child.child_by_field_name("right") else {
            continue;
        };

        let lhs = node_text(left, src);
        let is_module_exports =
            lhs == "module.exports" || lhs.starts_with("exports.");

        if !is_module_exports {
            continue;
        }

        // Determine what is being exported.
        let export_name = if lhs == "module.exports" {
            // Try to get the RHS name for reference target.
            match right.kind() {
                "identifier" => node_text(right, src),
                _ => lhs.clone(),
            }
        } else {
            // `exports.Foo = bar` → export name is "Foo"
            left.child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| lhs.clone())
        };

        refs.push(Ref {
            source_symbol_index: current_symbol_count,
            target_name: export_name,
            kind: EdgeKind::Imports,
            line: child.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// require() at the declarator level (outside a function body)
// ---------------------------------------------------------------------------

/// If `init_node` is `require('foo')`, push an Imports edge. Used for
/// top-level `const x = require('foo')` declarations.
fn try_emit_require(
    init_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    if init_node.kind() != "call_expression" {
        return;
    }
    let Some(func) = init_node.child_by_field_name("function") else {
        return;
    };
    let name = callee_name(func, src);
    if name != "require" {
        return;
    }
    if let Some(module) = extract_require_path(init_node, src) {
        refs.push(Ref {
            source_symbol_index,
            target_name: module.clone(),
            kind: EdgeKind::Imports,
            line: init_node.start_position().row as u32,
            module: Some(module),
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// for...in / for...of loop variable extraction
// ---------------------------------------------------------------------------

fn extract_for_loop_var(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    refs: &mut Vec<Ref>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;

    let Some(left) = node.child_by_field_name("left") else {
        return;
    };

    // `left` is typically `lexical_declaration` or bare `identifier`.
    let name = if left.kind() == "identifier" {
        node_text(left, src)
    } else {
        let mut found = String::new();
        let mut cur = left.walk();
        'outer: for child in left.children(&mut cur) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.kind() == "identifier" {
                        found = node_text(name_node, src);
                        break 'outer;
                    }
                }
            } else if child.kind() == "identifier" {
                found = node_text(child, src);
                break 'outer;
            }
        }
        found
    };

    if name.is_empty() {
        return;
    }

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
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: left.start_position().row as u32,
        end_line: left.end_position().row as u32,
        start_col: left.start_position().column as u32,
        end_col: left.end_position().column as u32,
        signature: Some(format!("const {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Emit a TypeRef to the iterable so the index builder can infer element type.
    if let Some(right) = node.child_by_field_name("right") {
        if right.kind() == "identifier" {
            let target = node_text(right, src);
            if !target.is_empty() {
                refs.push(Ref {
                    source_symbol_index: idx,
                    target_name: target,
                    kind: EdgeKind::TypeRef,
                    line: right.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// catch variable extraction
// ---------------------------------------------------------------------------

fn extract_catch_variable(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;

    // Locate the catch parameter — may be `catch_parameter` or bare `identifier`.
    let mut param_node: Option<Node> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "catch_parameter" | "identifier" => {
                param_node = Some(child);
                break;
            }
            _ => {}
        }
    }
    let Some(param) = param_node else {
        return;
    };

    let name_node = if param.kind() == "identifier" {
        param
    } else {
        let mut found: Option<Node> = None;
        let mut pcursor = param.walk();
        for child in param.children(&mut pcursor) {
            if child.kind() == "identifier" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return,
        }
    };

    let name = node_text(name_node, src);
    if name.is_empty() {
        return;
    }

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
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: name_node.start_position().row as u32,
        end_line: name_node.end_position().row as u32,
        start_col: name_node.start_position().column as u32,
        end_col: name_node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
