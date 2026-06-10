// =============================================================================
// parser/extractors/javascript/extract.rs  —  JavaScript / JSX extractor entry
//
// Owns the recursive AST walker and the declaration emitters (classes,
// functions, methods, fields, lexical/var declarations, destructuring,
// for-loop / catch bindings, class heritage). Each cross-cutting concern
// lives in its own sibling module:
//
//   - calls.rs    — call / new / JSX / tagged-template ref emission
//   - imports.rs  — ES module + CommonJS + prototype-install detection
//   - globals.rs  — top-level globals, IIFE/UMD descent, AngularJS DI,
//                   post-traversal type-identifier scan
// =============================================================================

use super::calls::{
    emit_call_ref_js, emit_new_ref_js, extract_calls, is_enclosing_function_parameter,
};
use super::globals::{harvest_top_level_globals, scan_all_type_identifiers};
use super::helpers::{detect_visibility, extract_jsdoc, node_text};
use super::imports::{
    extract_module_exports, extract_prototype_method, push_export_refs, push_import,
    try_emit_require,
};

use crate::parser::scope_tree::ScopeTree;
use crate::types::ExtractedSymbol;
use crate::types::{EdgeKind, ExtractedRef as Ref, ExtractedSymbol as Sym, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Detect minified or vendored library bundles that contribute massive
/// noise to the unresolved-refs metric without representing first-party
/// code (jQuery, typeahead.js, lodash, RxJS, generated theme bundles…).
///
/// Three signals, any one of which is sufficient:
///   1. Path ends in `.min.js` / `.min.mjs` / `.min.cjs` / `.bundle.js` —
///      the universal naming convention for minified bundles.
///   2. Source begins with a `/*!` preserve-comment header — the standard
///      Terser/UglifyJS marker that vendored libraries use to keep their
///      license through minification. Essentially never appears in
///      first-party application code.
///   3. The longest line in the first 16 KB exceeds 5 000 chars — catches
///      collapsed-IIFE bundles that lack the path or comment markers
///      (e.g. theme builds emitted by Webpack/Vite without the `/*!`
///      banner). The threshold is ~5× the longest line found in real
///      first-party test files, so false positives are vanishingly rare.
pub(super) fn looks_vendored_bundle(source: &str, file_path: &str) -> bool {
    let path_lower = file_path.to_ascii_lowercase();
    if path_lower.ends_with(".min.js")
        || path_lower.ends_with(".min.mjs")
        || path_lower.ends_with(".min.cjs")
        || path_lower.ends_with(".bundle.js")
    {
        return true;
    }

    if source.trim_start().starts_with("/*!") {
        return true;
    }

    let head_end = source.len().min(16 * 1024);
    // Walk back to a char boundary so we never slice through a multi-byte
    // UTF-8 codepoint.
    let mut end = head_end;
    while end > 0 && !source.is_char_boundary(end) {
        end -= 1;
    }
    let head = &source[..end];
    head.lines().any(|l| l.len() > 5000)
}

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
        ScopeKind {
            node_kind: "class_declaration",
            name_field: "name",
        },
        ScopeKind {
            node_kind: "function_declaration",
            name_field: "name",
        },
    ];

    let root = tree.root_node();
    let scope_tree = scope_tree::build(root, src_bytes, JS_SCOPE_KINDS);

    // Pre-pass: build a local-alias → module-path map from all import statements.
    let import_map = crate::ecosystem::ecmascript_imports::build_import_map(root, src_bytes);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<Ref> = Vec::new();

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

    // Apply ECMAScript import semantics to every ref via the shared
    // resolver — same module-of-truth as TS/JSX/TSX/Vue/Svelte/Astro.
    crate::ecosystem::imports::resolve_import_refs(&mut refs, &import_map);

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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
}

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
                            .and_then(|n| {
                                n.child_by_field_name("parameters").or_else(|| {
                                    // Single-param shorthand: `x => x` — the param is the
                                    // `identifier` child with field name "parameter".
                                    n.child_by_field_name("parameter")
                                })
                            })
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
                            byte_offset: 0,
                            declared_type: None,
                            return_type: None,
                            param_types: Vec::new(),
                            generic_params: Vec::new(),
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
                            byte_offset: 0,
                            declared_type: None,
                            return_type: None,
                            param_types: Vec::new(),
                            generic_params: Vec::new(),
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
                            byte_offset: 0,
                            declared_type: None,
                            return_type: None,
                            param_types: Vec::new(),
                            generic_params: Vec::new(),
                        });
                        // Recurse into the class body for methods/fields.
                        if let Some(init_node) = &init {
                            if let Some(body) = init_node.child_by_field_name("body") {
                                extract_js_node_inner(
                                    body,
                                    src,
                                    scope_tree,
                                    symbols,
                                    refs,
                                    Some(idx),
                                );
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
                            byte_offset: 0,
                            declared_type: None,
                            return_type: None,
                            param_types: Vec::new(),
                            generic_params: Vec::new(),
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
                                    extract_js_node(
                                        *init_node,
                                        src,
                                        scope_tree,
                                        symbols,
                                        refs,
                                        Some(idx),
                                    );
                                }
                                // Objects, arrays, template literals, etc. — recurse to
                                // find nested calls, declarations, and class bodies.
                                _ => {
                                    try_emit_require(init_node, src, idx, refs);
                                    extract_calls(init_node, src, idx, refs);
                                    extract_js_node(
                                        *init_node,
                                        src,
                                        scope_tree,
                                        symbols,
                                        refs,
                                        Some(idx),
                                    );
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
                                    &prop_name,
                                    &prop,
                                    src,
                                    scope_tree,
                                    symbols,
                                    parent_index,
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
                                            &prop_name,
                                            &val,
                                            src,
                                            scope_tree,
                                            symbols,
                                            parent_index,
                                            &scope_path,
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
                                            &rest_name,
                                            &rest_child,
                                            src,
                                            scope_tree,
                                            symbols,
                                            parent_index,
                                            &scope_path,
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
                                &elem_name,
                                &elem,
                                src,
                                scope_tree,
                                symbols,
                                parent_index,
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
                                        &rest_name,
                                        &rest_child,
                                        src,
                                        scope_tree,
                                        symbols,
                                        parent_index,
                                        &scope_path,
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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

// ---------------------------------------------------------------------------
// Class heritage refs
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
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::Inherits,
                            line: n.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: n.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                            col: 0,
                        });
                    }
                    "extends_clause" => {
                        let mut ec = n.walk();
                        for type_node in n.children(&mut ec) {
                            if type_node.kind() == "identifier" {
                                refs.push(Ref {
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: source_idx,
                                    target_name: node_text(type_node, src),
                                    kind: EdgeKind::Inherits,
                                    line: type_node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: type_node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                    col: 0,
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
            if !target.is_empty() && !is_enclosing_function_parameter(right, src, &target) {
                refs.push(Ref {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: idx,
                    target_name: target,
                    kind: EdgeKind::TypeRef,
                    line: right.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: right.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                    col: 0,
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
}

// build_import_map moved to crate::ecosystem::ecmascript_imports — both TS
// and JS extractors now share that single implementation.
