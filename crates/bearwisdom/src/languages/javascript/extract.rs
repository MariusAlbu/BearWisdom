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


use super::{helpers};
use super::helpers::{detect_visibility, extract_jsdoc, node_text};

use crate::languages::common::build_member_chain;
use crate::parser::scope_tree::ScopeTree;
use crate::types::{EdgeKind, ExtractedRef as Ref, ExtractedSymbol as Sym, SymbolKind};
use crate::types::{ExtractedRef, ExtractedSymbol};
use std::collections::HashMap;
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

    pub(crate) static JS_SCOPE_KINDS: &[ScopeKind] = &[
        ScopeKind { node_kind: "class_declaration", name_field: "name" },
        ScopeKind { node_kind: "function_declaration", name_field: "name" },
    ];

    let root = tree.root_node();
    let scope_tree = scope_tree::build(root, src_bytes, JS_SCOPE_KINDS);

    // Pre-pass: build a local-alias → module-path map from all import statements.
    let import_map = build_import_map(root, src_bytes);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_js_node(root, src_bytes, &scope_tree, &mut symbols, &mut refs, None);

    // Global-binding harvest: walk top-level statements (and the bodies of
    // top-level IIFEs) for assignments to `window.X`, `global.X`,
    // `globalThis.X`, `self.X`, `root.X`, or `this.X`. Each one registers
    // `X` as a file-top-level symbol so cross-file resolvers can find the
    // bindings that classic `<script src="…">` libraries install into the
    // browser global object (jQuery's `$`, Bootstrap's `bootstrap`,
    // AngularJS's `angular`, etc.). Without this, `jquery.js` produces
    // only function-scoped symbols and `$` stays unresolved project-wide.
    harvest_top_level_globals(root, src_bytes, &mut symbols);

    // Post-traversal full-tree scan: catch every type_identifier node that the
    // main walker missed (e.g. JSDoc-annotated variables, class heritage in
    // unusual positions, etc.).  JS has no type system so hits are sparse but
    // the scan is cheap and ensures coverage is symmetric with TypeScript.
    if !symbols.is_empty() {
        scan_all_type_identifiers(root, src_bytes, 0, &mut refs);
    }

    // Annotate call refs: if a Calls ref's target_name starts with a known
    // import alias (e.g. "UserService.findOne"), set module to the import source.
    if !import_map.is_empty() {
        annotate_call_modules_js(&mut refs, &import_map);
    }

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
                // Emit refs for the exported names (named exports, re-exports, default).
                let sym_idx = parent_index.unwrap_or_else(|| symbols.len());
                push_export_refs(&child, src, sym_idx, refs);
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
                // Extract calls from the field initializer value, if present.
                // The "value" field is the initializer expression itself, so we
                // need emit_call_ref_js / emit_new_ref_js for direct call nodes,
                // then extract_calls to pick up any nested calls in arguments.
                if let Some(value) = child.child_by_field_name("value") {
                    let sym_idx = parent_index.unwrap_or(0);
                    match value.kind() {
                        "call_expression" => {
                            emit_call_ref_js(&value, src, sym_idx, refs);
                            extract_calls(&value, src, sym_idx, refs);
                        }
                        "new_expression" => {
                            emit_new_ref_js(&value, src, sym_idx, refs);
                            extract_calls(&value, src, sym_idx, refs);
                        }
                        _ => {
                            extract_calls(&value, src, sym_idx, refs);
                        }
                    }
                }
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
            // `exports.X = ...` assignments, or ES5 prototype-method
            // installs (`Builder.prototype.withUrl = function () { … }`)
            // emitted by webpack/TS-to-ES5 transpilation.
            "expression_statement" => {
                extract_module_exports(&child, src, symbols.len(), refs);
                extract_prototype_method(&child, src, scope_tree, symbols, refs, parent_index);
                extract_js_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // Call expressions at any level not already handled by extract_calls
            // from inside a function/method body.  Captures top-level calls,
            // IIFE patterns, calls inside field initializers, etc.
            //
            // Use parent_index.unwrap_or(0) so the call is attributed to the
            // nearest enclosing named symbol or the first symbol in the file.
            "call_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                emit_call_ref_js(&child, src, sym_idx, refs);
                extract_js_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // `new Foo(...)` at module scope or in field initializers.
            "new_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                emit_new_ref_js(&child, src, sym_idx, refs);
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
                        // Extract calls and nested declarations from arrow body.
                        if let Some(init_node) = &init {
                            if let Some(body) = init_node.child_by_field_name("body") {
                                extract_calls(&body, src, idx, refs);
                                extract_js_node(body, src, scope_tree, symbols, refs, Some(idx));
                            } else {
                                // Expression-body arrow: `x => expr` — body IS the expr.
                                // Still emit calls if the body is a call/JSX expression.
                                extract_calls(init_node, src, idx, refs);
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
                                extract_js_node(body, src, scope_tree, symbols, refs, Some(idx));
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
                        if let Some(init_node) = &init {
                            match init_node.kind() {
                                // `const x = new Foo()` → Calls edge (JS convention)
                                "new_expression" => {
                                    emit_new_ref_js(init_node, src, idx, refs);
                                    extract_calls(init_node, src, idx, refs);
                                }
                                // `const x = require('foo')` → Imports edge.
                                // For `require`, also recurse into args in case of dynamic
                                // paths, but skip deep walk since it's just a string.
                                "call_expression" => {
                                    try_emit_require(init_node, src, idx, refs);
                                    // Still emit the call ref if it's not require().
                                    emit_call_ref_js(init_node, src, idx, refs);
                                    // Recurse into call arguments and body for nested decls.
                                    extract_calls(init_node, src, idx, refs);
                                    extract_js_node(*init_node, src, scope_tree, symbols, refs, Some(idx));
                                }
                                // Objects, arrays, template literals, etc. — recurse to
                                // find nested calls, declarations, and class bodies.
                                _ => {
                                    try_emit_require(init_node, src, idx, refs);
                                    extract_calls(init_node, src, idx, refs);
                                    extract_js_node(*init_node, src, scope_tree, symbols, refs, Some(idx));
                                }
                            }
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
                // Extract calls from the field initializer value, if present.
                if let Some(value) = child.child_by_field_name("value") {
                    let sym_idx = parent_index.unwrap_or(0);
                    match value.kind() {
                        "call_expression" => {
                            emit_call_ref_js(&value, src, sym_idx, refs);
                            extract_calls(&value, src, sym_idx, refs);
                        }
                        "new_expression" => {
                            emit_new_ref_js(&value, src, sym_idx, refs);
                            extract_calls(&value, src, sym_idx, refs);
                        }
                        _ => {
                            extract_calls(&value, src, sym_idx, refs);
                        }
                    }
                }
            }
            _ => {
                extract_js_node_inner(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

/// Emit refs for an `export_statement` node so the coverage system can match
/// at least one ref at the export statement's start line.
///
/// Handles:
/// - `export { foo, bar }` — emits Imports refs for each named specifier
/// - `export { foo as default } from './mod'` — same
/// - `export default expr` — emits Imports ref for the identifier/call name
/// - `export * from './mod'` — emits Imports ref for the module path
/// - `export const/function/class ...` — the inner decl handles symbols;
///   here we emit an Imports ref using the decl's name so the line is covered
/// - `export default { ... }` / `export default function() {}` — fallback ref
fn push_export_refs(node: &Node, src: &[u8], source_symbol_index: usize, refs: &mut Vec<Ref>) {
    let line = node.start_position().row as u32;
    let initial_ref_count = refs.len();

    let module_path = node.child_by_field_name("source").map(|s| {
        node_text(s, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `export { foo, bar }` or `export { foo } from './mod'`
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        // The exported name (after `as`, or the original name).
                        let exported = spec
                            .child_by_field_name("alias")
                            .or_else(|| spec.child_by_field_name("name"))
                            .map(|n| node_text(n, src))
                            .unwrap_or_default();
                        if !exported.is_empty() {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: exported,
                                kind: EdgeKind::Imports,
                                line: spec.start_position().row as u32,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                }
            }

            // `export * from './mod'` — the `*` child is a namespace_export or literal
            "namespace_export" => {
                if let Some(mod_path) = &module_path {
                    refs.push(Ref {
                        source_symbol_index,
                        target_name: mod_path.clone(),
                        kind: EdgeKind::Imports,
                        line,
                        module: module_path.clone(),
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }

            // `export default <identifier>` — the exported identifier
            "identifier" => {
                let name = node_text(child, src);
                if name != "default" && name != "export" && !name.is_empty() {
                    refs.push(Ref {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }

            // `export default callExpr(...)` — emit a call ref for the callee
            "call_expression" => {
                emit_call_ref_js(&child, src, source_symbol_index, refs);
            }

            // `export default new Foo()` — emit a new ref
            "new_expression" => {
                emit_new_ref_js(&child, src, source_symbol_index, refs);
            }

            // `export const/let/var X = ...` and `export function foo()` etc.
            // Emit an Imports ref using the first declared name so the line is covered.
            "lexical_declaration" | "variable_declaration" => {
                let mut dc = child.walk();
                'outer_lex: for decl in child.children(&mut dc) {
                    if decl.kind() == "variable_declarator" {
                        if let Some(name_node) = decl.child_by_field_name("name") {
                            let name = node_text(name_node, src);
                            if !name.is_empty() {
                                refs.push(Ref {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::Imports,
                                    line,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                });
                                break 'outer_lex;
                            }
                        }
                    }
                }
            }

            "function_declaration" | "generator_function_declaration" => {
                // `export function foo() {}` / `export default function foo() {}`
                // Re-export form (`export { foo } from './mod'`) is handled in
                // the `export_clause` arm above; those land here only when an
                // anonymous default function is the export target, which we
                // skip — the inner function symbol already covers the line.
                //
                // For the NAMED form, emitting a self-targeted Imports ref is
                // pure noise: the function symbol exists in the same file,
                // there's no module path to resolve against, and the ref
                // always lands in `unresolved_refs` since name-collision with
                // `init` / `main` / etc. across unrelated files makes the
                // heuristic pick the wrong target (fluentui-blazor has 26+
                // unrelated `init` symbols across .cs / .razor.js files).
                //
                // Skip. Coverage comes from the function symbol itself.
                if module_path.is_some() {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if !name.is_empty() {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::Imports,
                                line,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                }
            }

            "class_declaration" | "class" => {
                // Same rationale as `function_declaration` above — only emit
                // for re-export forms where the module path gives the
                // resolver something to match against.
                if module_path.is_some() {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if !name.is_empty() {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::Imports,
                                line,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                }
            }

            _ => {}
        }
    }

    // Fallback: if we emitted no ref yet (e.g. `export * from './mod'` without a
    // namespace_export child, `export default {}`, `export default function() {}`),
    // emit an Imports ref at the export line using the module path or a placeholder.
    if refs.len() == initial_ref_count {
        let target = module_path
            .clone()
            .unwrap_or_else(|| "default".to_string());
        refs.push(Ref {
            source_symbol_index,
            target_name: target.clone(),
            kind: EdgeKind::Imports,
            line,
            module: module_path,
            chain: None,
            byte_offset: 0,
        });
    }
}

fn push_import(node: &Node, src: &[u8], current_symbol_count: usize, refs: &mut Vec<Ref>) {
    let module_path = node.child_by_field_name("source").map(|s| {
        node_text(s, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    });

    let line = node.start_position().row as u32;

    // Always emit one Imports ref at the import statement's start line using the
    // module path. This ensures the coverage budget for `(import_statement, line)`
    // is satisfied even when all named specifiers appear on subsequent lines.
    if let Some(mod_path) = &module_path {
        refs.push(Ref {
            source_symbol_index: current_symbol_count,
            target_name: mod_path.clone(),
            kind: EdgeKind::Imports,
            line,
            module: module_path.clone(),
            chain: None,
            byte_offset: 0,
        });
    }

    let initial_ref_count = refs.len();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            let mut ic = child.walk();
            for item in child.children(&mut ic) {
                match item.kind() {
                    // `import React from 'react'` — default import
                    "identifier" => {
                        refs.push(Ref {
                            source_symbol_index: current_symbol_count,
                            target_name: node_text(item, src),
                            kind: EdgeKind::TypeRef,
                            line: item.start_position().row as u32,
                            module: module_path.clone(),
                            chain: None,
                            byte_offset: 0,
                        });
                    }
                    // `import { useState, useEffect } from 'react'`
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
                                    byte_offset: 0,
                                });
                            }
                        }
                    }
                    // `import * as ns from 'module'` — namespace import
                    "namespace_import" => {
                        // The local binding is the identifier after `as`.
                        let mut nc = item.walk();
                        for ns_child in item.children(&mut nc) {
                            if ns_child.kind() == "identifier" {
                                refs.push(Ref {
                                    source_symbol_index: current_symbol_count,
                                    target_name: node_text(ns_child, src),
                                    kind: EdgeKind::TypeRef,
                                    line: ns_child.start_position().row as u32,
                                    module: module_path.clone(),
                                    chain: None,
                                    byte_offset: 0,
                                });
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Fallback: side-effect imports like `import './styles.css'` have no import_clause.
    // Emit an Imports ref using the module path so the line is covered.
    if refs.len() == initial_ref_count {
        if let Some(mod_path) = &module_path {
            refs.push(Ref {
                source_symbol_index: current_symbol_count,
                target_name: mod_path.clone(),
                kind: EdgeKind::Imports,
                line,
                module: module_path.clone(),
                chain: None,
                byte_offset: 0,
            });
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
                            byte_offset: 0,
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
                                    byte_offset: 0,
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
                    // Build a structured chain; fall back to plain callee name.
                    let chain = build_member_chain(func_node, src);
                    let callee = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name(func_node, src));

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
                                byte_offset: 0,
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
                                byte_offset: 0,
                            });
                        }
                    }
                    // Regular call — emit with chain for member access resolution.
                    else if !callee.is_empty() {
                        // Parameter-shadow filter — see `emit_call_ref_js`
                        // comment for rationale. Receiver-first on chains
                        // of ≥2 segments; callee-only for bare calls.
                        let receiver_name = chain
                            .as_ref()
                            .filter(|c| c.segments.len() >= 2)
                            .and_then(|c| c.segments.first())
                            .map(|s| s.name.as_str());
                        let shadowed = match receiver_name {
                            Some(recv) => is_enclosing_function_parameter(func_node, src, recv),
                            None => is_enclosing_function_parameter(func_node, src, &callee),
                        };
                        if !shadowed {
                            crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: callee,
                                kind: EdgeKind::Calls,
                                line: func_node.start_position().row as u32,
                                module: None,
                                chain,
                                byte_offset: func_node.start_byte() as u32,
                            });
                        }
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
                            byte_offset: 0,
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
                            byte_offset: 0,
                        });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            // JSX: `<Component />`, `<Component>...</Component>`, or the
            // member-expression form `<Foo.Bar />` that React Context uses
            // for `<MyContext.Provider value={x}>`. Emit a Calls edge only
            // for user-defined components (PascalCase first char). Lowercase
            // tags like `<div>` / `<span>` are HTML intrinsics — not graph-
            // resolvable symbols — and were previously emitted as `type_ref`
            // refs that polluted `unresolved_refs`.
            //
            // For the member-expression form we emit the TAIL segment name
            // (`Provider`) as `target_name` with a structured MemberChain
            // `[MyContext, Provider]` so the chain walker can resolve the
            // receiver's inferred React.Context<T> type to the Provider
            // member. Previously we stuffed the full `"MyContext.Provider"`
            // string into `target_name` — which never resolved against any
            // symbol and polluted unresolved_refs on every .jsx file that
            // used the Context pattern. Matches the TypeScript extractor.
            "jsx_self_closing_element" | "jsx_opening_element" => {
                let tag = child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0));
                if let Some(tag_node) = tag {
                    let tag_name = node_text(tag_node, src);
                    let is_component = tag_name.chars().next().map_or(false, |c| c.is_uppercase());
                    if !tag_name.is_empty() && is_component {
                        let chain = build_member_chain(tag_node, src);
                        let target = chain
                            .as_ref()
                            .and_then(|c| c.segments.last())
                            .map(|s| s.name.clone())
                            .unwrap_or(tag_name);
                        crate::languages::emit_chain_type_ref(
                            &chain,
                            source_symbol_index,
                            &tag_node,
                            refs,
                        );
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line: tag_node.start_position().row as u32,
                            module: None,
                            chain,
                            byte_offset: tag_node.start_byte() as u32,
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
            // Return ONLY the property name (the last segment), matching the
            // TypeScript extractor convention and the `ExtractedRef.target_name`
            // contract ("For chain-bearing refs, this is the LAST segment name").
            //
            // The prior implementation concatenated `obj.prop` using the raw
            // text of the object sub-tree. That was catastrophic for any
            // chain whose receiver was itself a call expression (Chai / Jasmine
            // assertions):
            //   expect(scratch.innerHTML).to.equal
            // was stored as a single target_name of literally that whole
            // string, inflating `unresolved_refs` by thousands of rows per
            // test-heavy project (javascript-preact alone: ~2,500 such refs).
            //
            // The resolver key is the method name; receiver context should
            // come from chain walking, not from stuffing it into target_name.
            node.child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(node, src))
        }
        // Unwrap parens and recurse: `(foo.bar)()` → `bar`.
        "parenthesized_expression" => node
            .named_child(0)
            .map(|inner| callee_name(inner, src))
            .unwrap_or_default(),
        // Keyword-shaped callees: `import('mod')` (dynamic import) and
        // `super(...)` both parse with a keyword node as the call's
        // function. The call sites downstream branch on the string value
        // (`"import"` → Imports edge; `"super"` → filtered as a keyword).
        "import" | "super" => node_text(node, src),
        // IIFEs (`(function(){})()`, `(() => {})()`) and other dynamic
        // callees have no named target — emitting a ref for them used to
        // dump the whole function body source into `target_name` via the
        // `rsplit('.')` fallback, producing garbage like:
        //     "data;\n            });\n        }\n\n        init();\n    }\n})"
        // that polluted `unresolved_refs` (SimplCommerce alone: ~100 such
        // rows from AngularJS module IIFEs). No target name ⇒ no ref.
        _ => String::new(),
    }
}

/// Emit a Calls ref for a single `call_expression` node.
///
/// Mirrors the TypeScript `calls::emit_call_ref` but uses the JS-local
/// `callee_name` helper and handles `require`/`import` as Imports edges.
fn emit_call_ref_js(
    call_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    let Some(func_node) = call_node.child_by_field_name("function") else {
        return;
    };
    // Build a structured chain first; fall back to plain callee name when the
    // node isn't chainable (e.g. an anonymous arrow-function callee).
    let chain = build_member_chain(func_node, src);
    let callee = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| callee_name(func_node, src));

    if callee == "require" {
        if let Some(module) = extract_require_path(call_node, src) {
            refs.push(Ref {
                source_symbol_index,
                target_name: module.clone(),
                kind: EdgeKind::Imports,
                line: call_node.start_position().row as u32,
                module: Some(module),
                chain: None,
                byte_offset: 0,
            });
        }
    } else if callee == "import" {
        if let Some(module) = extract_first_string_arg(call_node, src) {
            refs.push(Ref {
                source_symbol_index,
                target_name: module.clone(),
                kind: EdgeKind::Imports,
                line: call_node.start_position().row as u32,
                module: Some(module),
                chain: None,
                byte_offset: 0,
            });
        }
    } else if !callee.is_empty() && !is_js_keyword(&callee) {
        // Parameter-shadow filter: `(setter) => setter(e.target.value)`
        // emits a Calls ref for `setter` that can never resolve because
        // the binding is a local parameter, not a declared function. Same
        // for chain receivers — `(function ($, currentSearchOption) { …
        // currentSearchOption.hasOwnProperty(k) … })` puts
        // `currentSearchOption` in `unresolved_refs` as a TypeRef. Drop
        // both forms when the target matches an enclosing parameter.
        let receiver_name = chain
            .as_ref()
            .filter(|c| c.segments.len() >= 2)
            .and_then(|c| c.segments.first())
            .map(|s| s.name.as_str());
        if let Some(recv) = receiver_name {
            if is_enclosing_function_parameter(func_node, src, recv) {
                return;
            }
        } else if is_enclosing_function_parameter(func_node, src, &callee) {
            return;
        }
        // Emit a TypeRef for the chain receiver when it looks like a type name.
        crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
        refs.push(Ref {
            source_symbol_index,
            target_name: callee,
            kind: EdgeKind::Calls,
            line: func_node.start_position().row as u32,
            module: None,
            chain,
            byte_offset: func_node.start_byte() as u32,
        });
    }
}

/// JS reserved words that should never be emitted as call targets.
/// Tree-sitter-javascript produces `super(...)` as a `call_expression` whose
/// `function` field is literally the identifier `super`. That leaks into
/// `unresolved_refs` as a `super` target (450+ refs in javascript-preact
/// alone). Similarly for `import(...)`, `new.target`, etc. — all keywords
/// the resolver has no business looking up against the symbol index.
fn is_js_keyword(name: &str) -> bool {
    matches!(
        name,
        "super" | "this" | "new" | "typeof" | "instanceof" | "void"
            | "yield" | "await" | "delete" | "in" | "of" | "return"
            | "throw" | "try" | "catch" | "finally" | "debugger"
            | "if" | "else" | "switch" | "case" | "default" | "break"
            | "continue" | "for" | "while" | "do" | "function" | "class"
            | "extends" | "const" | "let" | "var" | "static" | "async"
            | "true" | "false" | "null" | "undefined"
    )
}

/// True when `name` is bound as a parameter of any enclosing function in
/// the AST (function declaration, function expression, arrow function,
/// method, generator). Used to filter unresolved-ref emission for chain
/// walkers whose receiver / callee / iterable is a local parameter rather
/// than a global type or function — the resolver has nothing to point it
/// at and `unresolved_refs` would only carry noise.
///
/// Walks the parent chain from `at` up to the program root. Handles
/// destructured (`{ a, b }`), array (`[x, y]`), rest (`...rest`), and
/// default (`x = 1`) parameter forms.
fn is_enclosing_function_parameter(at: Node, src: &[u8], name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut cur = at;
    while let Some(parent) = cur.parent() {
        if matches!(
            parent.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "generator_function_declaration"
                | "generator_function"
        ) {
            let params = parent
                .child_by_field_name("parameters")
                .or_else(|| parent.child_by_field_name("parameter"));
            if let Some(params) = params {
                if parameter_list_binds(params, src, name) {
                    return true;
                }
            }
        }
        cur = parent;
    }
    false
}

/// True when `name` appears as a binding identifier in a function
/// parameter list node (`formal_parameters`, or a single `identifier` in
/// the arrow-short form `x => …`). Recurses through patterns to handle
/// destructuring, rest, and defaults.
fn parameter_list_binds(params: Node, src: &[u8], name: &str) -> bool {
    if pattern_binds_name(params, src, name) {
        return true;
    }
    let mut cursor = params.walk();
    for child in params.named_children(&mut cursor) {
        if pattern_binds_name(child, src, name) {
            return true;
        }
    }
    false
}

/// Recursive walk of a binding pattern to check whether `name` appears
/// as one of the introduced identifiers. Covers:
///   - plain identifier:     `x`
///   - object pattern:       `{ a, b: c, ...rest }`
///   - array pattern:        `[x, y, ...z]`
///   - rest pattern:         `...rest`
///   - default-value:        `x = 1`
///   - assignment pattern:   `{ a = 1 }`
///   - typed params:         tree-sitter-typescript wraps these but the
///                           JS grammar also accepts them in .tsx files.
fn pattern_binds_name(node: Node, src: &[u8], name: &str) -> bool {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            node_text(node, src) == name
        }
        "rest_pattern" | "spread_element" => {
            // `...rest` — the identifier lives under the rest marker.
            node.named_child(0)
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        "assignment_pattern" => {
            // `x = default` — check the left (the bound name).
            node.child_by_field_name("left")
                .or_else(|| node.named_child(0))
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        "object_pattern" | "array_pattern" | "object_assignment_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if pattern_binds_name(child, src, name) {
                    return true;
                }
            }
            false
        }
        "pair_pattern" => {
            // `{ a: b }` — the bound name is the VALUE, not the key.
            node.child_by_field_name("value")
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        "required_parameter" | "optional_parameter" | "formal_parameters" => {
            let inner = node
                .child_by_field_name("pattern")
                .or_else(|| node.named_child(0));
            inner
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Emit a Calls ref for a single `new_expression` node.
fn emit_new_ref_js(
    new_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    let Some(constructor) = new_node.child_by_field_name("constructor") else {
        return;
    };
    let name = callee_name(constructor, src);
    if !name.is_empty() {
        refs.push(Ref {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::Calls,
            line: constructor.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
        });
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
            // `module.exports = SomeIdentifier` — link the export to that
            // symbol. For non-identifier RHS (`module.exports = factory()`,
            // `= { … }`, `= someCall()`) there's no concrete target to
            // point at; emitting the literal "module.exports" as Imports
            // target is just noise in `unresolved_refs`. Skip silently.
            match right.kind() {
                "identifier" => node_text(right, src),
                _ => continue,
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
            byte_offset: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// ES5 prototype-method installs (webpack / TS-to-ES5 transpile output)
// ---------------------------------------------------------------------------

/// Detect `X.prototype.Y = function (…) { … }` (or arrow) assignments and
/// emit `Y` as a Method under qualified name `X.Y`. This is how ES5-style
/// classes install instance methods — SignalR's `signalr.js`, any code that
/// targets `lib: ES5`, and most TypeScript `target: "es5"` transpile output
/// land on this pattern. Without it, the chain walker sees the constructor
/// function `X` but none of its methods, so `new X().foo()` leaves `foo`
/// unresolved.
fn extract_prototype_method(
    stmt_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    refs: &mut Vec<Ref>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;

    let mut cursor = stmt_node.walk();
    for child in stmt_node.children(&mut cursor) {
        if child.kind() != "assignment_expression" {
            continue;
        }
        let Some(left) = child.child_by_field_name("left") else { continue };
        let Some(right) = child.child_by_field_name("right") else { continue };

        // Left must be a member_expression: {object: member_expression, property: identifier}.
        if left.kind() != "member_expression" {
            continue;
        }
        let Some(outer_object) = left.child_by_field_name("object") else { continue };
        let Some(outer_property) = left.child_by_field_name("property") else { continue };
        if outer_property.kind() != "property_identifier"
            && outer_property.kind() != "identifier"
        {
            continue;
        }
        if outer_object.kind() != "member_expression" {
            continue;
        }
        // Inner member's property must be "prototype" and its object a plain identifier.
        let Some(inner_object) = outer_object.child_by_field_name("object") else { continue };
        let Some(inner_property) = outer_object.child_by_field_name("property") else { continue };
        if inner_object.kind() != "identifier" {
            continue;
        }
        if node_text(inner_property, src) != "prototype" {
            continue;
        }

        // Right must be a function-like expression. Bail on everything else
        // so we don't misattribute field initializers (`.prototype.x = []`).
        let is_callable = matches!(
            right.kind(),
            "function_expression" | "arrow_function" | "generator_function"
        );
        if !is_callable {
            continue;
        }

        let class_name = node_text(inner_object, src);
        let method_name = node_text(outer_property, src);
        if class_name.is_empty() || method_name.is_empty() {
            continue;
        }

        let parent_scope = if stmt_node.start_byte() > 0 {
            scope_tree::find_scope_at(scope_tree, stmt_node.start_byte() - 1)
        } else {
            None
        };
        let scope_path = scope_tree::scope_path(parent_scope);

        let kind = if method_name == "constructor" {
            SymbolKind::Constructor
        } else {
            SymbolKind::Method
        };

        let idx = symbols.len();
        symbols.push(Sym {
            name: method_name.clone(),
            qualified_name: format!("{class_name}.{method_name}"),
            kind,
            visibility: None,
            start_line: outer_property.start_position().row as u32,
            end_line: right.end_position().row as u32,
            start_col: outer_property.start_position().column as u32,
            end_col: right.end_position().column as u32,
            signature: Some(format!("{class_name}.prototype.{method_name} = function")),
            doc_comment: extract_jsdoc(stmt_node, src),
            scope_path,
            parent_index,
        });

        // Harvest calls inside the function body so in-method refs attach
        // to the method symbol rather than the enclosing scope.
        if let Some(body) = right.child_by_field_name("body") {
            extract_calls(&body, src, idx, refs);
        }
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
            byte_offset: 0,
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
            // Parameter-shadow filter: `for (k in currentSearchOption)`
            // where `currentSearchOption` is an IIFE / outer-function
            // parameter — the iterable binds to the parameter value at
            // runtime, not to any declared symbol, so the TypeRef only
            // pollutes unresolved_refs with no possible resolution target.
            if !target.is_empty()
                && !is_enclosing_function_parameter(right, src, &target)
            {
                refs.push(Ref {
                    source_symbol_index: idx,
                    target_name: target,
                    kind: EdgeKind::TypeRef,
                    line: right.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
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
// Import map + call module annotation
// ---------------------------------------------------------------------------

/// Build a map of `local_alias → module_path` from all top-level import
/// statements in the JavaScript file.
///
/// Handles all three import forms:
/// - `import Foo from './bar'`            → `"Foo" → "./bar"`
/// - `import { Foo, Bar as B } from ...`  → `"Foo" → ..., "B" → ...`
/// - `import * as ns from './bar'`        → `"ns" → "./bar"`
fn build_import_map(root: Node, src: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "import_statement" {
            continue;
        }
        let Some(module_node) = child.child_by_field_name("source") else {
            continue;
        };
        let module_path = node_text(module_node, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if module_path.is_empty() {
            continue;
        }

        let mut ic = child.walk();
        for clause in child.children(&mut ic) {
            if clause.kind() != "import_clause" {
                continue;
            }
            let mut cc = clause.walk();
            for item in clause.children(&mut cc) {
                match item.kind() {
                    // `import Foo from './bar'` — default import
                    "identifier" => {
                        let local = node_text(item, src);
                        if !local.is_empty() {
                            map.insert(local, module_path.clone());
                        }
                    }
                    // `import { Foo, Bar as B } from './bar'`
                    "named_imports" => {
                        let mut ni = item.walk();
                        for spec in item.children(&mut ni) {
                            if spec.kind() != "import_specifier" {
                                continue;
                            }
                            // `alias` is the local name when `as` is used; otherwise
                            // `name` is both the exported and local name.
                            let local = spec
                                .child_by_field_name("alias")
                                .or_else(|| spec.child_by_field_name("name"))
                                .map(|n| node_text(n, src))
                                .unwrap_or_default();
                            if !local.is_empty() {
                                map.insert(local, module_path.clone());
                            }
                        }
                    }
                    // `import * as ns from './bar'`
                    "namespace_import" => {
                        let mut nc = item.walk();
                        for ns_child in item.children(&mut nc) {
                            if ns_child.kind() == "identifier" {
                                let local = node_text(ns_child, src);
                                if !local.is_empty() {
                                    map.insert(local, module_path.clone());
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    map
}

/// Annotate `Calls` refs whose `target_name` is a qualified member access
/// (e.g. `"UserService.findOne"`) with the module of the first segment when
/// that segment is a known import alias.
///
/// The JS extractor encodes member calls as `"obj.method"` in `target_name`
/// with `chain: None`. We split on the first `.` to extract the object name.
fn annotate_call_modules_js(refs: &mut Vec<ExtractedRef>, import_map: &HashMap<String, String>) {
    for r in refs.iter_mut() {
        if r.kind != EdgeKind::Calls || r.module.is_some() {
            continue;
        }
        // Try the chain-based path first (in case chain is ever populated).
        if let Some(chain) = &r.chain {
            if chain.segments.len() >= 2 {
                let first = &chain.segments[0].name;
                if let Some(module_path) = import_map.get(first) {
                    r.module = Some(module_path.clone());
                    continue;
                }
            }
        }
        // Fall back to splitting the dotted target_name.
        if r.target_name.contains('.') {
            if let Some(prefix) = r.target_name.splitn(2, '.').next() {
                if let Some(module_path) = import_map.get(prefix) {
                    r.module = Some(module_path.clone());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type_identifier scanner
// ---------------------------------------------------------------------------

/// Recursively scan ALL descendants of `node` for `type_identifier` nodes and
/// emit a `TypeRef` for each one found.
///
/// JavaScript has no type system, so hits are rare (JSDoc-annotated bindings,
/// class heritage identifiers), but the scan is cheap and ensures parity with
/// the TypeScript extractor.
fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" && child.is_named() {
            let name = node_text(child, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: crate::types::EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
        // Recurse into ALL children regardless.
        scan_all_type_identifiers(child, src, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Top-level global-binding harvest
//
// Classic browser-distributed libraries (`<script src="jquery.js">`,
// `angular.js`, `bootstrap.js`, …) wrap their bodies in an IIFE and
// install their public API by assigning to the global object:
//
//     (function(root, factory) {
//         /* … */
//         root.jQuery = root.$ = factory();
//     })(typeof window !== "undefined" ? window : this, function() { … });
//
// Without special handling, a vanilla extract walks the IIFE body and
// emits function-scoped symbols that no cross-file resolver can match
// against `$` / `jQuery` calls in project code. This pass fishes out
// those global assignments and emits them as file-top-level symbols so
// the Tier-1 resolver finds them alongside ordinary top-level
// declarations.
//
// Recognised LHS shapes (object part of `member_expression` or
// `subscript_expression`):
//
//     * `identifier` with name in {window, global, globalThis, self, root}
//     * `this`  (top-level `this.X = …` in sloppy-mode UMD wrappers)
//     * IIFE parameter name that was bound to `window` / `global` via the
//       IIFE's argument list (handled implicitly: all IIFEs have their
//       first parameter treated as a global receiver)
// ---------------------------------------------------------------------------

fn harvest_top_level_globals(root: Node, src: &[u8], symbols: &mut Vec<Sym>) {
    // Pass 1: top-level statements (window.X, IIFE root.X, this.X).
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        scan_statement_for_globals(child, src, &[], symbols);
    }
    // Pass 2: AngularJS DI registrations anywhere in the AST.
    // Patterns like `angular.module(...).service('Upload', ...)` or
    // `ngFileUpload.factory('Foo', [...])` register DI tokens that are
    // consumed across files. Without this, consumer code referencing
    // `Upload.upload(...)` leaves `Upload` unresolved.
    scan_angular_registrations(root, src, symbols);
}

/// AngularJS module-registration methods. Map each to the SymbolKind that
/// best describes what consumers can do with the DI token:
///   - service/factory/provider/component/controller → class-like (has
///     methods, often used as chain receivers → `Upload.upload(...)`).
///   - directive/filter → function-like (invoked directly or as a tag).
///   - value/constant → variable (opaque value).
///   - decorator/run/config → not DI tokens; skip.
fn angular_registration_kind(method: &str) -> Option<SymbolKind> {
    match method {
        "service" | "factory" | "provider" | "component" | "controller" => {
            Some(SymbolKind::Class)
        }
        "directive" | "filter" => Some(SymbolKind::Function),
        "value" | "constant" => Some(SymbolKind::Variable),
        _ => None,
    }
}

/// Walk the AST recursively and emit a top-level symbol for each AngularJS
/// DI registration of the form `X.service('Name', …)`, `.factory(...)`,
/// `.filter(...)`, etc. The string-literal first argument becomes the
/// symbol name; the SymbolKind is chosen by `angular_registration_kind`.
fn scan_angular_registrations(node: Node, src: &[u8], symbols: &mut Vec<Sym>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            try_emit_angular_registration(&child, src, symbols);
        }
        scan_angular_registrations(child, src, symbols);
    }
}

fn try_emit_angular_registration(call: &Node, src: &[u8], symbols: &mut Vec<Sym>) {
    let Some(func) = call.child_by_field_name("function") else { return };
    if func.kind() != "member_expression" {
        return;
    }
    let Some(prop) = func.child_by_field_name("property") else { return };
    let method = node_text(prop, src);
    let Some(kind) = angular_registration_kind(&method) else { return };

    let Some(args) = call.child_by_field_name("arguments") else { return };
    let mut acursor = args.walk();
    let Some(first_arg) = args.named_children(&mut acursor).next() else { return };
    if first_arg.kind() != "string" {
        return;
    }
    let Some(name) = string_literal_value(first_arg, src) else { return };
    push_typed_global_symbol(&name, kind, call, symbols);
}

/// Unwrap a string-literal node's text into its content, rejecting empty
/// strings and content that doesn't look like an identifier/DI-token name
/// (to avoid polluting the index with arbitrary string literals).
fn string_literal_value(node: Node, src: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    // Prefer the inner string_fragment node (tree-sitter-javascript parses
    // "foo" as string { string_fragment "foo" }).
    let mut cursor = node.walk();
    let content = node
        .named_children(&mut cursor)
        .find(|c| c.kind() == "string_fragment")
        .map(|c| node_text(c, src))
        .unwrap_or_else(|| {
            let raw = node_text(node, src);
            raw.trim_matches(|c| c == '"' || c == '\'').to_string()
        });
    if content.is_empty() {
        return None;
    }
    // Accept names that look like identifiers or DI tokens ($http, ng-click,
    // etc.). Reject anything with whitespace / quotes / slashes to avoid
    // stuffing arbitrary literals ("api/products", "/path/to") in symbols.
    let ok = content
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '-'));
    if !ok {
        return None;
    }
    Some(content)
}

fn push_typed_global_symbol(
    name: &str,
    kind: SymbolKind,
    anchor: &Node,
    symbols: &mut Vec<Sym>,
) {
    // Dedup: if a top-level symbol with the same (name, kind) already exists,
    // skip. But allow multiple kinds for the same name (a name may exist as
    // both Class (service) and Variable (window alias)).
    if symbols
        .iter()
        .any(|s| s.name == name && s.parent_index.is_none() && s.kind == kind)
    {
        return;
    }
    let line = anchor.start_position().row as u32;
    symbols.push(Sym {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(crate::types::Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

/// Examine one statement node (or the body of a recursively-walked IIFE)
/// for global assignments. `alias_globals` is the set of identifier names
/// that should be treated as "the global object" inside the current
/// scope — starts empty at the file level and is populated with the
/// IIFE's first parameter name when we descend into one.
fn scan_statement_for_globals(
    node: Node,
    src: &[u8],
    alias_globals: &[String],
    symbols: &mut Vec<Sym>,
) {
    match node.kind() {
        "expression_statement" => {
            if let Some(inner) = node.named_child(0) {
                match inner.kind() {
                    "assignment_expression" => {
                        try_emit_global_assignment(&inner, src, alias_globals, symbols);
                    }
                    "call_expression" => {
                        // Top-level IIFE — recurse into its body with the
                        // first parameter promoted to a global alias.
                        descend_iife(&inner, src, alias_globals, symbols);
                    }
                    // `(function(){…}(args))` — the UMD form where arguments
                    // are INSIDE the outer parens. The call_expression lives
                    // one level deeper than the `(function(){})(args)` form.
                    "parenthesized_expression" => {
                        if let Some(call) = inner.named_child(0) {
                            if call.kind() == "call_expression" {
                                descend_iife(&call, src, alias_globals, symbols);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // `var jQuery = window.jQuery = ...` / `var $ = jQuery` at top level —
        // the main extractor already captures these. Nothing to do here.
        _ => {}
    }
}

/// If `call` is an IIFE (`(function(){…})()` / `(()=>{})()`), walk its
/// body's top-level statements. The IIFE's first parameter name is added
/// to `alias_globals` so assignments like `root.X = …` inside register
/// globals too.
///
/// Also pairs each parameter with its call-site argument. When an argument
/// is `this` / `window` / `global` / `globalThis` / `self`, the parameter
/// joins `alias_globals`. When an argument is a string literal, it is
/// recorded so UMD-style `root[paramName] = factory()` assignments can
/// resolve the subscript and emit the literal as a global (the slugify /
/// jQuery UMD template).
fn descend_iife(
    call: &Node,
    src: &[u8],
    alias_globals: &[String],
    symbols: &mut Vec<Sym>,
) {
    let Some(func) = call.child_by_field_name("function") else { return };
    let inner_func = match func.kind() {
        "parenthesized_expression" => match func.named_child(0) {
            Some(n) => n,
            None => return,
        },
        "function_expression" | "arrow_function" => func,
        _ => return,
    };
    if !matches!(
        inner_func.kind(),
        "function_expression" | "arrow_function"
    ) {
        return;
    }
    // Collect parameter names (positional).
    let param_names: Vec<String> = inner_func
        .child_by_field_name("parameters")
        .map(|params| {
            let mut pcursor = params.walk();
            params
                .named_children(&mut pcursor)
                .map(|p| match p.kind() {
                    "identifier" => node_text(p, src),
                    "required_parameter" | "optional_parameter" | "formal_parameters" => p
                        .child_by_field_name("pattern")
                        .or_else(|| p.named_child(0))
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| node_text(n, src))
                        .unwrap_or_default(),
                    _ => String::new(),
                })
                .collect()
        })
        .unwrap_or_default();

    // Collect call-site argument nodes (positional).
    let args: Vec<Node> = call
        .child_by_field_name("arguments")
        .map(|args_node| {
            let mut ac = args_node.walk();
            args_node.named_children(&mut ac).collect()
        })
        .unwrap_or_default();

    // Build the alias/binding state. Any parameter bound to a root-like
    // value becomes a global alias. Any parameter bound to a string literal
    // is recorded for UMD subscript resolution.
    let mut new_aliases: Vec<String> = alias_globals.to_vec();
    let mut string_bindings: HashMap<String, String> = HashMap::new();

    for (i, pname) in param_names.iter().enumerate() {
        if pname.is_empty() {
            continue;
        }
        // Preserve legacy behavior: the FIRST parameter always becomes a
        // global alias (the historical jQuery / Bootstrap IIFE pattern has
        // no args and relies on parameter-position convention).
        if i == 0 {
            new_aliases.push(pname.clone());
        }
        let Some(arg) = args.get(i) else { continue };
        match arg.kind() {
            "this" => {
                if !new_aliases.contains(pname) {
                    new_aliases.push(pname.clone());
                }
            }
            "identifier" => {
                let ident = node_text(*arg, src);
                if matches!(ident.as_str(), "window" | "global" | "globalThis" | "self" | "root") {
                    if !new_aliases.contains(pname) {
                        new_aliases.push(pname.clone());
                    }
                }
            }
            "string" => {
                if let Some(lit) = string_literal_value(*arg, src) {
                    string_bindings.insert(pname.clone(), lit);
                }
            }
            _ => {}
        }
    }

    let Some(body) = inner_func.child_by_field_name("body") else { return };

    // UMD subscript pass: scan for `root[name] = factory()` where `root` is
    // a global alias and `name` is a parameter bound to a string literal.
    if !string_bindings.is_empty() {
        scan_umd_subscript_exports(body, src, &new_aliases, &string_bindings, symbols);
    }

    // Existing top-level-assignment pass inside the IIFE body.
    let mut bcursor = body.walk();
    for child in body.named_children(&mut bcursor) {
        scan_statement_for_globals(child, src, &new_aliases, symbols);
    }
}

/// Walk an IIFE body and emit globals for UMD subscript assignments like
/// `root[name] = factory()` where `root` is an IIFE parameter bound to a
/// root-like value (`this`, `window`, …) and `name` is an IIFE parameter
/// bound to a string literal passed at the call site. These are the
/// canonical UMD export pattern for classic libraries (slugify, dayjs-style
/// single-file modules, etc.).
fn scan_umd_subscript_exports(
    node: Node,
    src: &[u8],
    alias_globals: &[String],
    string_bindings: &HashMap<String, String>,
    symbols: &mut Vec<Sym>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "assignment_expression" {
            try_emit_umd_subscript(&child, src, alias_globals, string_bindings, symbols);
        }
        scan_umd_subscript_exports(child, src, alias_globals, string_bindings, symbols);
    }
}

fn try_emit_umd_subscript(
    assign: &Node,
    src: &[u8],
    alias_globals: &[String],
    string_bindings: &HashMap<String, String>,
    symbols: &mut Vec<Sym>,
) {
    let Some(left) = assign.child_by_field_name("left") else { return };
    if left.kind() != "subscript_expression" {
        return;
    }
    let Some(object) = left.child_by_field_name("object") else { return };
    if object.kind() != "identifier" {
        return;
    }
    let object_name = node_text(object, src);
    if !alias_globals.iter().any(|a| a == &object_name) {
        return;
    }
    let Some(index) = left.child_by_field_name("index") else { return };
    if index.kind() != "identifier" {
        return;
    }
    let index_name = node_text(index, src);
    let Some(literal) = string_bindings.get(&index_name) else { return };
    // Emit as Function: UMD exports are almost always callable (factory()).
    push_typed_global_symbol(literal, SymbolKind::Function, assign, symbols);
}

/// Recognise `obj.X = Y` / `obj.X = obj.Y = Z` and emit every property
/// name on the LHS (including chained assignments) as a top-level global.
fn try_emit_global_assignment(
    assign: &Node,
    src: &[u8],
    alias_globals: &[String],
    symbols: &mut Vec<Sym>,
) {
    let Some(left) = assign.child_by_field_name("left") else { return };
    let Some(right) = assign.child_by_field_name("right") else { return };

    if lhs_targets_global(&left, src, alias_globals) {
        if let Some(name) = member_property_name(&left, src) {
            push_global_symbol(&name, assign, symbols);
        }
    }

    // Chained assignment: RHS may itself be an assignment_expression.
    if right.kind() == "assignment_expression" {
        try_emit_global_assignment(&right, src, alias_globals, symbols);
    }
}

/// True when the LHS of an assignment resolves to a property of a known
/// global object — directly (`window.X`), via an IIFE parameter alias
/// (`root.X` when `root` was the IIFE's first parameter), or via
/// top-level `this` (UMD wrappers in sloppy mode).
fn lhs_targets_global(lhs: &Node, src: &[u8], alias_globals: &[String]) -> bool {
    let (object, _) = match lhs.kind() {
        "member_expression" | "subscript_expression" => (
            match lhs.child_by_field_name("object") {
                Some(o) => o,
                None => return false,
            },
            lhs.kind(),
        ),
        _ => return false,
    };
    match object.kind() {
        "identifier" => {
            let name = node_text(object, src);
            matches!(name.as_str(), "window" | "global" | "globalThis" | "self" | "root")
                || alias_globals.iter().any(|a| a == &name)
        }
        "this" => true,
        _ => false,
    }
}

/// Extract the string property name from a member or subscript LHS.
fn member_property_name(lhs: &Node, src: &[u8]) -> Option<String> {
    match lhs.kind() {
        "member_expression" => lhs
            .child_by_field_name("property")
            .map(|n| node_text(n, src))
            .filter(|s| !s.is_empty()),
        "subscript_expression" => {
            // `foo["X"]` — only accept string-literal indices.
            let idx = lhs.child_by_field_name("index")?;
            if idx.kind() != "string" {
                return None;
            }
            let raw = node_text(idx, src);
            let trimmed = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))?;
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn push_global_symbol(name: &str, anchor: &Node, symbols: &mut Vec<Sym>) {
    // Deduplicate against symbols already pushed in this file. The main
    // extractor may have captured the same name via another path (unlikely
    // for globals inside IIFE bodies, but cheap insurance).
    if symbols.iter().any(|s| s.name == name && s.parent_index.is_none()) {
        return;
    }
    let line = anchor.start_position().row as u32;
    symbols.push(Sym {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: crate::types::SymbolKind::Variable,
        visibility: Some(crate::types::Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

