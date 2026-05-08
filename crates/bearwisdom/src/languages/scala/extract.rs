// =============================================================================
// parser/extractors/scala/mod.rs  —  Scala symbol and reference extractor
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use super::calls::extract_calls_from_body;
use super::decorators::{extract_case_class_params, extract_decorators, extract_match_patterns};
use super::helpers::{call_target_name, classify_class, node_text};
use super::symbols::{
    extract_enum_body, extract_extends_with, push_export, push_extension_definition,
    push_function_def, push_given_definition, push_import, push_package_clause, push_type_def,
    push_type_definition, push_val_var, recurse_body,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static SCALA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_definition",    name_field: "name" },
    ScopeKind { node_kind: "object_definition",   name_field: "name" },
    ScopeKind { node_kind: "trait_definition",    name_field: "name" },
    ScopeKind { node_kind: "enum_definition",     name_field: "name" },
    ScopeKind { node_kind: "function_definition", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Scala grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    // A top-level brace-less `package foo.bar` puts every subsequent top-level
    // declaration into that package, but tree-sitter exposes those siblings
    // outside the package_clause AST node, so the scope-tree walker never sees
    // the package as an enclosing scope. Hoist the package name once, prefix
    // the scope tree (so nested-class qnames pick it up automatically), then
    // fix up the truly-top-level symbols at the end (covers brace-form too,
    // where parent_index is set but enclosing_scope is None).
    let hoisted_pkg = hoist_top_level_package(root, src);

    let mut scope_tree = scope_tree::build(root, src, SCALA_SCOPE_KINDS);
    if let Some(pkg) = hoisted_pkg.as_deref() {
        for entry in scope_tree.iter_mut() {
            entry.qualified_name = format!("{pkg}.{}", entry.qualified_name);
        }
    }

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, &mut symbols, &mut refs, None);

    scope_tree::prefix_top_level_qnames(&mut symbols, hoisted_pkg.as_deref());

    // Post-traversal: scan the entire CST for type_identifier nodes and emit
    // TypeRef for any that the top-down walker didn't reach (e.g., inside
    // interpolated expressions, complex type projections, or error subtrees).
    scan_all_type_refs(root, src, &mut refs);

    // Post-filter: drop TypeRefs whose target is a generic type parameter in
    // scope at the ref's line. `class HealthRoutes[F[_]: Monad]` puts F in
    // scope across the class body; uses inside (`def get: F[Response]`) emit
    // TypeRef for F that the resolver can never match against a real symbol.
    // Mirrors the TS (23a4055) and Kotlin (9cb5717) post-filters.
    {
        let mut scopes: Vec<(String, u32, u32)> = Vec::new();
        collect_type_param_scopes(root, src, &mut scopes);
        if !scopes.is_empty() {
            refs.retain(|r| {
                if r.kind != crate::types::EdgeKind::TypeRef {
                    return true;
                }
                !scopes.iter().any(|(name, start, end)| {
                    &r.target_name == name && r.line >= *start && r.line <= *end
                })
            });
        }
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

/// Hoist top-level brace-less `package foo.bar` declarations into a single
/// dotted prefix. Returns None if the file has no top-level package or only
/// brace-form `package foo { ... }` (which the scope tree handles via byte
/// ranges).
///
/// Chained brace-less packages — `package foo` followed by `package bar` at
/// file top level — concatenate to `foo.bar`.
fn hoist_top_level_package(root: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "package_clause" {
            continue;
        }
        // brace-form has a `body` field; skip — scope tree covers it via the
        // package_clause's byte range once the body is recursed into below.
        if child.child_by_field_name("body").is_some() {
            continue;
        }
        let mut cc = child.walk();
        for inner in child.children(&mut cc) {
            if matches!(
                inner.kind(),
                "stable_id" | "identifier" | "package_identifier"
            ) {
                parts.push(super::helpers::node_text(inner, src));
                break;
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

/// Walk every `type_parameters` node and record each declared type-parameter
/// name plus the line range of its declaring parent (class / trait / object
/// / method / function / type_definition). Uses of that name inside the
/// range are generic-binding references, not external type refs.
///
/// Scala tree-sitter exposes a variety of type-parameter shapes depending on
/// variance (`+T` / `-T`), bounds (`T <: Upper`), context bounds
/// (`T: TypeClass`), and higher-kindedness (`F[_]`). The name is always the
/// first `type_identifier` / `identifier` child of the type-parameter node,
/// regardless of shape — so we just scan for either.
fn collect_type_param_scopes(
    node: tree_sitter::Node,
    src: &[u8],
    out: &mut Vec<(String, u32, u32)>,
) {
    // tree-sitter-scala exposes `type_parameters` two different ways:
    //
    //   * As a named FIELD on declarations that the grammar authors chose
    //     to label — class_definition, trait_definition, enum_definition,
    //     given_definition, extension_definition, type_definition,
    //     function_type, lambda_expression, type_lambda.
    //   * As a direct KIND child (unlabeled) on every other declaration
    //     that accepts them — most importantly `function_definition` and
    //     `function_declaration`, where type params like `def two[F, G]`
    //     live.
    //
    // Checking the field alone misses the unlabeled case (and vice-versa).
    // Look for both on every node so uses of `G` inside `def two[G]` get
    // scoped correctly.
    let mut tp_node: Option<tree_sitter::Node> = node.child_by_field_name("type_parameters");
    if tp_node.is_none() {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_parameters" {
                tp_node = Some(child);
                break;
            }
        }
    }
    if let Some(tp) = tp_node {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        collect_type_param_names(&tp, src, start_line, end_line, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_param_scopes(child, src, out);
    }
}

/// Pull the parameter names out of a `type_parameters` subtree. The grammar
/// varies by variance and bound shape (`+T`, `-T`, `T`, `F[_]`, etc.), but
/// the name is always the first `identifier` / `type_identifier` descendant
/// of each direct child.
fn collect_type_param_names(
    tp_node: &tree_sitter::Node,
    src: &[u8],
    start_line: u32,
    end_line: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = tp_node.walk();
    for tp in tp_node.children(&mut cursor) {
        if !tp.is_named() {
            continue;
        }
        // Find the first identifier descendant — that's the parameter name.
        if let Some(name) = first_identifier_descendant(&tp, src) {
            out.push((name, start_line, end_line));
        }
    }
}

/// Left-to-right, pre-order descendant scan for the first `identifier` /
/// `type_identifier`. Must be pre-order left-first so that for a shape like
/// `G[_]: Trace` we return `G` (the parameter name, leftmost) and not
/// `Trace` (a bound, further down the subtree).
fn first_identifier_descendant(node: &tree_sitter::Node, src: &[u8]) -> Option<String> {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        if let Ok(name) = node.utf8_text(src) {
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = first_identifier_descendant(&child, src) {
            return Some(name);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

pub(super) fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                push_import(&child, src, symbols.len(), refs);
            }

            "class_definition" => {
                let kind = classify_class(&child, src);
                let idx = push_type_def(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                    // Extract case class constructor params as Property symbols.
                    let qname = symbols[sym_idx].qualified_name.clone();
                    extract_case_class_params(&child, src, sym_idx, &qname, symbols);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "object_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Namespace, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "trait_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Scala 3 enum
            "enum_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                extract_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Abstract method declaration in trait/class (no body).
            "function_declaration" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                // Extract TypeRef from return type and parameter types in declarations.
                if let Some(sym_idx) = idx {
                    extract_type_refs_from_function(&child, src, sym_idx, refs);
                }
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    // Extract TypeRef from return type and parameter types.
                    extract_type_refs_from_function(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        // If the body IS a match_expression (e.g. `def f = x match {...}`),
                        // extract patterns directly; extract_calls_from_body only sees children.
                        if body.kind() == "match_expression" {
                            extract_match_patterns(&body, src, sym_idx, refs);
                        }
                        // For expression-body functions (`def f = expr`), the body may be an
                        // infix_expression or call_expression directly — handle the root too.
                        dispatch_body_node(body, src, sym_idx, refs);
                        extract_calls_from_body(&body, src, sym_idx, refs);
                        // Recurse into the body with extract_node so that nested val/var/def
                        // definitions inside blocks are extracted as symbols, and infix/call
                        // expressions in deeply-nested blocks are visited.
                        extract_node(body, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                }
            }

            "val_definition" | "var_definition" | "val_declaration" | "var_declaration" => {
                // Extract type annotation *before* pushing the symbol (so we have the right index).
                let sym_idx = if let Some(type_node) = child.child_by_field_name("type") {
                    // For declarations, use parent_index; for definitions, we'll use the symbol we just created.
                    let idx_to_use = match child.kind() {
                        "val_definition" | "var_definition" => symbols.len(), // Will be the index of the symbol we push below
                        _ => parent_index.unwrap_or(0), // For declarations
                    };
                    push_val_var(&child, src, scope_tree, symbols, parent_index);
                    extract_type_refs_from_type_node(&type_node, src, idx_to_use, refs);
                    idx_to_use
                } else {
                    let idx = symbols.len();
                    push_val_var(&child, src, scope_tree, symbols, parent_index);
                    idx
                };
                // Recurse into the value expression for nested val/var/def definitions
                // (e.g. `val x = { val inner = ...; inner }`) and call edges.
                if matches!(child.kind(), "val_definition" | "var_definition") {
                    if let Some(value_node) = child.child_by_field_name("value") {
                        extract_calls_from_body(&value_node, src, sym_idx, refs);
                        extract_node(value_node, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    // Type inference from initializer: if no explicit type annotation,
                    // infer from constructor call. `val repo = Repository()` → TypeRef "Repository".
                    if child.child_by_field_name("type").is_none() {
                        if let Some(value_node) = child.child_by_field_name("value") {
                            infer_type_from_value(&value_node, src, sym_idx, refs);
                        }
                    }
                }
            }

            // Scala `type` alias / abstract type member.
            "type_definition" | "type_declaration" => {
                push_type_definition(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // Scala 3 `given` — implicit instance.
            "given_definition" => {
                let idx = push_given_definition(&child, src, scope_tree, symbols, refs, parent_index);
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Scala 3 `extension` — extension methods block.
            "extension_definition" => {
                let idx = push_extension_definition(&child, src, scope_tree, symbols, parent_index);
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // `package foo.bar { ... }` — emit a Namespace symbol and recurse.
            // Also handles `package foo.bar` (no body) by emitting the symbol only.
            "package_clause" => {
                let pkg_idx = push_package_clause(&child, src, scope_tree, symbols, parent_index);
                let effective_parent = pkg_idx.or(parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, effective_parent);
                } else {
                    // Scala `package foo.bar` at top-level with no braces — the rest
                    // of the file is implicitly in scope; recurse treating siblings
                    // as children (caller handles this via the main loop).
                    let mut cc = child.walk();
                    for inner in child.children(&mut cc) {
                        match inner.kind() {
                            "class_definition" | "object_definition" | "trait_definition"
                            | "enum_definition" | "function_definition" | "function_declaration"
                            | "val_definition" | "var_definition" | "import_declaration" => {
                                extract_node(inner, src, scope_tree, symbols, refs, effective_parent);
                            }
                            _ => {}
                        }
                    }
                }
            }

            // `export foo._` / `export foo.{Bar, Baz}` — emit Imports refs.
            "export_declaration" => {
                push_export(&child, src, symbols.len(), refs);
            }

            // extends_clause and with_clause are handled by extract_extends_with
            // when processing the parent class/trait/object/enum.  When they appear
            // as children of any other node (e.g. in a nested class inside a function
            // body), fall through to explicit type-ref walking so no edges are missed.
            "extends_clause" | "with_clause" => {
                if let Some(sym_idx) = parent_index {
                    super::symbols::extract_extends_with_node(&child, src, sym_idx, refs);
                }
            }

            "match_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_match_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // for-expression / for-comprehension — extract embedded calls and type refs.
            "for_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_calls_from_body(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // Call expressions outside a function body (e.g. val/var initializers,
            // top-level statements, object body expressions).
            // Also recurse with extract_node to capture val_definition / function_definition
            // inside lambda bodies passed as arguments (especially Scala 3 indented blocks).
            "call_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                dispatch_body_node(child, src, sym_idx, refs);
                extract_calls_from_body(&child, src, sym_idx, refs);
                // Pick up nested symbols (val/def) inside lambda argument bodies.
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "infix_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                dispatch_body_node(child, src, sym_idx, refs);
                extract_calls_from_body(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // `new Dog(args)` or `new Trait { def method() = ... }` at expression level.
            // Extract calls AND recurse into any anonymous class body for nested symbols.
            "instance_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_calls_from_body(&child, src, sym_idx, refs);
                // Recurse to extract nested function_definition / val_definition inside
                // anonymous class bodies: `new Foo { def bar() = ... }`.
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // Generic type arguments appearing in expression context (e.g. method call
            // type parameters, or a generic type used as a value).  Walk them for
            // nested type_identifier nodes.
            "type_arguments" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_type_refs_from_type_node(&child, src, sym_idx, refs);
            }

            // A bare type_identifier in expression context (e.g. pattern matching,
            // companion object reference, generic type position).
            "type_identifier" => {
                let sym_idx = parent_index.unwrap_or(0);
                let name = helpers::node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Type reference extraction helpers
// ---------------------------------------------------------------------------

/// Extract TypeRef edges from a type annotation node (e.g., the `: String` part).
/// Recursively handles generic_type, compound_type, etc.
/// NOTE: We extract ALL type identifiers, including builtins. Filtering happens in resolution.
fn extract_type_refs_from_type_node(
    type_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Handle the type_node itself if it's a type_identifier.
    if type_node.kind() == "type_identifier" {
        let name = helpers::node_text(*type_node, src);
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: crate::types::EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        return;
    }

    let mut cursor = type_node.walk();
    for child in type_node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" => {
                let name = helpers::node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
            "generic_type" => {
                // Recurse into generic_type to find type_identifier and type_arguments.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            "type_arguments" => {
                // Recurse into type arguments (e.g., `List[User]` → process `User`).
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            "compound_type" | "annotated_type" | "with_type" => {
                // Recurse into compound types.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            "function_type" => {
                // Function types may have parameter and return type nodes.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            _ => {
                // Recurse into other node types to find nested type_identifier nodes.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract TypeRef edges from function parameter and return types.
fn extract_type_refs_from_function(
    func_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Extract return type.
    if let Some(ret_type) = func_node.child_by_field_name("return_type") {
        extract_type_refs_from_type_node(&ret_type, src, source_symbol_index, refs);
    }

    // Extract parameter types.
    if let Some(params) = func_node.child_by_field_name("parameters") {
        extract_type_refs_from_type_node(&params, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Dispatch a single expression node that may be the direct body of a function
/// (i.e. not a block container). Handles infix_expression and call_expression
/// that would otherwise be missed because `extract_calls_from_body` only walks
/// children of the passed node.
fn dispatch_body_node(
    node: tree_sitter::Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "infix_expression" => {
            if let Some(op) = node.child_by_field_name("operator") {
                let target_name = node_text(op, src);
                if !target_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: crate::types::EdgeKind::Calls,
                        line: op.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }
        "call_expression" => {
            if let Some(callee) = node
                .child_by_field_name("function")
                .or_else(|| node.named_child(0))
            {
                let chain = calls::build_chain(&callee, src);
                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| call_target_name(&callee, src));
                if !target_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: crate::types::EdgeKind::Calls,
                        line: callee.start_position().row as u32,
                        module: None,
                        chain,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }
        // `new Dog(args)` as a direct function body expression.
        "instance_expression" => {
            let mut ic = node.walk();
            for inner in node.children(&mut ic) {
                match inner.kind() {
                    "type_identifier" => {
                        let name = node_text(inner, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: crate::types::EdgeKind::Calls,
                                line: inner.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                        }
                    }
                    "stable_type_identifier" => {
                        let full = node_text(inner, src);
                        let simple = full.rsplit('.').next().unwrap_or(&full).to_string();
                        if !simple.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: simple,
                                kind: crate::types::EdgeKind::Calls,
                                line: inner.start_position().row as u32,
                                module: Some(full),
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type reference scan
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit TypeRef edges for every `type_identifier` node
/// found. This catches type references that the top-down walker misses due to
/// structural gaps (e.g., type parameters in nested positions, string
/// interpolation types, or any node kind not explicitly handled above).
///
/// Deduplication is handled at resolution — over-emitting is always safe.
fn scan_all_type_refs(node: tree_sitter::Node, src: &[u8], refs: &mut Vec<ExtractedRef>) {
    scan_type_refs_inner(node, src, 0, refs);
}

fn scan_type_refs_inner(
    node: tree_sitter::Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "type_identifier" {
        let name = helpers::node_text(node, src);
        if !name.is_empty() && !super::predicates::is_scala_primitive_type(&name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: crate::types::EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        // type_identifier is a leaf — no children to recurse into.
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_type_refs_inner(child, src, source_symbol_index, refs);
    }
}

/// Infer the type of a val/var from its initializer expression when no explicit
/// type annotation is present. Emits a TypeRef that feeds `field_type_name`.
///
/// Handles:
///   `val repo = Repository()` → TypeRef to "Repository"
///   `val svc = ServiceImpl(config)` → TypeRef to "ServiceImpl"
///   `val x = foo()` → skipped (lowercase = not a type constructor)
fn infer_type_from_value(
    value_node: &Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match value_node.kind() {
        "call_expression" => {
            if let Some(func) = value_node.child_by_field_name("function") {
                let type_name = match func.kind() {
                    "identifier" => {
                        let name = helpers::node_text(func, src);
                        if name.starts_with(|c: char| c.is_uppercase()) {
                            Some(name)
                        } else {
                            None
                        }
                    }
                    // `Foo.apply()` or `Foo.Bar()`
                    "field_expression" => {
                        if let Some(chain) = calls::build_chain(&func, src) {
                            chain.segments.last()
                                .filter(|s| s.name.starts_with(|c: char| c.is_uppercase()))
                                .map(|s| s.name.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = type_name {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: value_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }
        // `val x = SomeObject` — direct reference to a singleton/companion.
        "identifier" => {
            let name = helpers::node_text(*value_node, src);
            if name.starts_with(|c: char| c.is_uppercase()) {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: crate::types::EdgeKind::TypeRef,
                    line: value_node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        // `new Repository()` — explicit instantiation (Scala 2 style).
        "instance_expression" => {
            // First named child is typically the type name.
            if let Some(type_node) = value_node.named_child(0) {
                let name = helpers::node_text(type_node, src);
                if name.starts_with(|c: char| c.is_uppercase()) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: value_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

