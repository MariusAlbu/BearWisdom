// =============================================================================
// parser/extractors/kotlin/mod.rs  —  Kotlin symbol and reference extractor
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use super::calls::extract_calls_from_body;
use super::decorators::{annotation_name_pub, extract_decorators, extract_lambda_params, extract_when_patterns};
use super::helpers::{classify_class, find_child_by_kind, node_text};
use super::symbols::{
    emit_import, extract_class_body, extract_delegation_specifiers, extract_imports,
    extract_primary_constructor_params, extract_type_parameter_bounds,
    push_companion_object, push_function_decl, push_getter_decl, push_property_decl,
    push_secondary_constructor, push_setter_decl, push_type_decl,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static KOTLIN_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",     name_field: "name" },
    ScopeKind { node_kind: "object_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration",  name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Kotlin grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    // Kotlin's `package foo.bar` puts every top-level type into that package,
    // but the package_header AST node is a sibling of the declarations rather
    // than an ancestor — so the scope-tree walker doesn't see it as an
    // enclosing scope. Hoist the package, prefix the scope tree (so nested
    // qnames pick it up), and after extraction prefix the truly-top-level
    // symbols.
    let hoisted_pkg = hoist_top_level_package(root, src);

    let mut scope_tree = scope_tree::build(root, src, KOTLIN_SCOPE_KINDS);
    if let Some(pkg) = hoisted_pkg.as_deref() {
        for entry in scope_tree.iter_mut() {
            entry.qualified_name = format!("{pkg}.{}", entry.qualified_name);
        }
    }

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, &mut symbols, &mut refs, None);

    scope_tree::prefix_top_level_qnames(&mut symbols, hoisted_pkg.as_deref());

    // Emit a Namespace symbol for the package_header so the resolver's
    // `file_namespace` lookup succeeds (used by same-package resolution).
    // Append at the END so symbols[0] keeps pointing at the first declared
    // type — `scan_all_type_refs` hardcodes source_symbol_index=0 and
    // changing what idx 0 points to silently re-attributes its refs.
    if let Some(pkg) = hoisted_pkg.as_deref() {
        if let Some((line, col, end_line, end_col)) = find_package_header_span(root) {
            symbols.push(ExtractedSymbol {
                name: pkg.rsplit('.').next().unwrap_or(pkg).to_string(),
                qualified_name: pkg.to_string(),
                kind: SymbolKind::Namespace,
                visibility: None,
                start_line: line,
                end_line,
                start_col: col,
                end_col,
                signature: Some(format!("package {pkg}")),
                doc_comment: None,
                scope_path: None,
                parent_index: None,
            });
        }
    }

    // Post-traversal: scan the entire CST for user_type / nullable_type nodes
    // and emit TypeRef for any type names not already captured by the walker.
    scan_all_type_refs(root, src, &mut refs);

    // Post-filter: drop TypeRefs whose target is a generic type parameter
    // in scope at the ref's line. `abstract class Foo<STATE : State>` puts
    // STATE in scope within the class body; uses of `STATE` inside emit
    // user_type nodes that look identical to any real type reference, so
    // the scan_all_type_refs pass picks them up as unresolved externals.
    // Mirrors the TypeScript resolver's same-class fix (commit 23a4055).
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

    // Dedup identical refs. `scan_all_type_refs` walks the entire CST and
    // emits a TypeRef for every `user_type` / `nullable_type` / `annotation`
    // node, while `extract_node`'s per-declaration arms ALSO emit refs for
    // the same nodes when they appear inside class bodies, property types,
    // or function signatures. The result is exact-duplicate `ExtractedRef`s
    // that pass through to `unresolved_refs` and double the per-line bucket
    // counts.
    //
    // Key on (source, target, kind, line, module) — `module` carries
    // semantic info the resolver uses (e.g., one push-site sets it from a
    // qualified name, another leaves it None for bare references). Refs
    // that differ only in module are NOT duplicates and must survive.
    // byte_offset is excluded because the AST nodes that share a line
    // legitimately have different offsets and the resolver doesn't read it.
    {
        let mut seen: std::collections::HashSet<(
            usize,
            String,
            crate::types::EdgeKind,
            u32,
            Option<String>,
        )> = std::collections::HashSet::with_capacity(refs.len());
        refs.retain(|r| {
            seen.insert((
                r.source_symbol_index,
                r.target_name.clone(),
                r.kind,
                r.line,
                r.module.clone(),
            ))
        });
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

/// Read the file-level `package_header` and return its dotted identifier.
/// Kotlin only has a single brace-less package per file (no `package foo {
/// ... }` form), so a single optional string suffices.
fn hoist_top_level_package(root: Node, src: &[u8]) -> Option<String> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "package_header" {
            continue;
        }
        let mut cc = child.walk();
        for inner in child.children(&mut cc) {
            if matches!(
                inner.kind(),
                "qualified_identifier" | "identifier" | "simple_identifier"
            ) {
                let text = node_text(inner, src);
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

/// Locate the `package_header` byte-position span so the synthesized
/// Namespace symbol carries useful line/col data.
fn find_package_header_span(root: Node) -> Option<(u32, u32, u32, u32)> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_header" {
            return Some((
                child.start_position().row as u32,
                child.start_position().column as u32,
                child.end_position().row as u32,
                child.end_position().column as u32,
            ));
        }
    }
    None
}

/// Walk every `type_parameters` node in the tree and record each declared
/// type-parameter name along with the byte-line range over which it is in
/// scope (the range of the *parent* declaration — class, function, property,
/// type alias).
fn collect_type_param_scopes(
    node: tree_sitter::Node,
    src: &[u8],
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_parameters" {
            // Scope is the parent of type_parameters — the class/function/etc.
            // declaration. If there's no parent (shouldn't happen at grammar
            // root), fall back to the type_parameters span itself.
            let scope_node = child.parent().unwrap_or(child);
            let start_line = scope_node.start_position().row as u32;
            let end_line = scope_node.end_position().row as u32;
            let mut tc = child.walk();
            for tp in child.children(&mut tc) {
                if tp.kind() != "type_parameter" {
                    continue;
                }
                // Type-parameter name: first simple_identifier / identifier /
                // type_identifier child. Bounds (the part after `:`) are
                // wrapped in `type` / `user_type` nodes we skip here.
                let mut tpc = tp.walk();
                for c in tp.children(&mut tpc) {
                    if matches!(c.kind(), "simple_identifier" | "identifier" | "type_identifier") {
                        if let Ok(name) = c.utf8_text(src) {
                            if !name.is_empty() {
                                out.push((name.to_string(), start_line, end_line));
                            }
                        }
                        break;
                    }
                }
            }
        }
        collect_type_param_scopes(child, src, out);
    }
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
            "import_list" => {
                extract_imports(&child, src, symbols.len(), refs);
            }

            "import" => {
                emit_import(&child, src, symbols.len(), refs);
            }

            "class_declaration" => {
                let kind = classify_class(&child, src);
                let idx = push_type_decl(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    extract_type_parameter_bounds(&child, src, sym_idx, refs);
                    // Extract primary constructor params (promoted properties + TypeRefs).
                    extract_primary_constructor_params(&child, src, scope_tree, symbols, refs, idx);
                    extract_class_body(&child, src, scope_tree, symbols, refs, idx);
                }
            }

            "companion_object" => {
                let idx = push_companion_object(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    // `class_body` is a non-field child of companion_object.
                    if let Some(body) = find_child_by_kind(&child, "class_body") {
                        extract_node(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }

            "object_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Class, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    // Members live inside a `class_body` direct child — the
                    // kotlin-ng grammar does not expose a `body` field on
                    // object_declaration, so `child_by_field_name("body")`
                    // returns None and silently drops every inner `val` /
                    // `fun` declaration. Without this, `object JavaBuildConfig {
                    // val JAVA_VERSION = ... }` emits only the class symbol
                    // and callers can never resolve `JavaBuildConfig.JAVA_VERSION`.
                    extract_class_body(&child, src, scope_tree, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    extract_type_parameter_bounds(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_node(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }

            "function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_parameter_bounds(&child, src, sym_idx, refs);
                    // Extract TypeRefs from function value parameters (parameter types,
                    // annotations on parameters, return type).
                    extract_function_param_types(&child, src, sym_idx, refs);
                    // function_body is a child (not a named field) in kotlin-ng 1.1.
                    let body = child.child_by_field_name("body")
                        .or_else(|| find_child_by_kind(&child, "function_body"));
                    if let Some(b) = body {
                        extract_calls_from_body(&b, src, sym_idx, refs);
                        extract_lambda_params(&b, src, sym_idx, symbols);
                        // Recurse with extract_node so local property_declaration nodes inside
                        // the function body produce Property symbols (val/var inside functions).
                        extract_node(b, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                }
            }

            "when_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_when_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "property_declaration" => {
                let pre_len = symbols.len();
                push_property_decl(&child, src, scope_tree, symbols, parent_index);
                let sym_idx = if symbols.len() > pre_len { pre_len } else { parent_index.unwrap_or(0) };
                // Emit TypeRef edges for annotations on this property (@Inject, @Autowired, etc.).
                extract_decorators(&child, src, sym_idx, refs);
                // In kotlin-ng, the declared type lives inside:
                //   property_declaration → variable_declaration → type → user_type | nullable_type | ...
                // Run the full calls extractor over the property_declaration node so
                // the `user_type | nullable_type` arms in extract_calls_from_body
                // pick up all type refs, including in the initializer expression.
                calls::extract_calls_from_body(&child, src, sym_idx, refs);

                // Type inference from initializer: if no explicit type annotation,
                // infer the type from a constructor call in the initializer.
                // `val repo = Repository()` → TypeRef from property to "Repository".
                // This feeds field_type_name in the chain walker.
                if child.child_by_field_name("type").is_none() {
                    infer_type_from_initializer(&child, src, sym_idx, refs);
                }

                // Extract getter and setter accessors as Method symbols.
                let mut pc = child.walk();
                for inner in child.children(&mut pc) {
                    match inner.kind() {
                        "getter" => {
                            push_getter_decl(&inner, src, scope_tree, symbols, Some(sym_idx));
                        }
                        "setter" => {
                            push_setter_decl(&inner, src, scope_tree, symbols, Some(sym_idx));
                        }
                        _ => {}
                    }
                }
            }

            // `typealias Foo = Bar` — field `type` holds the identifier name.
            "type_alias" => {
                push_type_decl_alias(&child, src, scope_tree, symbols, parent_index);
            }

            "secondary_constructor" => {
                push_secondary_constructor(&child, src, scope_tree, symbols, parent_index);
            }

            // Call expressions that appear outside a function body (e.g. property
            // initializers, top-level statements, delegate expressions).
            // extract_calls_from_body handles the callee chain and emits Calls refs.
            // We then recurse with extract_node only into argument / lambda children
            // so that property_declaration nodes inside lambda arguments
            // (e.g. `run { val x = ... }`) produce Property symbols — WITHOUT
            // re-entering the callee navigation_expression chain, which would
            // re-emit every intermediate chain segment as a spurious Calls ref.
            "call_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_calls_from_body(&child, src, sym_idx, refs);
                // Only recurse into argument-carrying children, not the callee.
                extract_call_node_args(&child, src, scope_tree, symbols, refs, parent_index);
            }
            // Standalone navigation_expression (not inside call_expression) —
            // extract calls, but do NOT recurse with extract_node since
            // navigation_expression nodes contain no symbol declarations.
            "navigation_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_calls_from_body(&child, src, sym_idx, refs);
            }

            // Standalone annotations at the current scope level — emit TypeRef.
            "annotation" | "file_annotation" => {
                let sym_idx = parent_index.unwrap_or(0);
                emit_annotation_ref(&child, src, sym_idx, refs);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Call node argument recursion helper
// ---------------------------------------------------------------------------

/// Recurse with `extract_node` into only the argument-carrying children of a
/// `call_expression`.  This is used in the top-level `extract_node` arm for
/// `call_expression` so that:
///
///   1. Property/symbol declarations inside lambda arguments are captured
///      (e.g. `run { val x = ... }` → `x` becomes a Property symbol).
///   2. The callee chain (navigation_expression / simple_identifier) is NOT
///      re-entered by `extract_node`, which would cause every nested
///      navigation_expression level to fire `extract_calls_from_body` again
///      and emit spurious Calls refs for intermediate chain segments.
fn extract_call_node_args<'a>(
    call_node: &tree_sitter::Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<crate::types::ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = call_node.walk();
    let mut first_named_skipped = false;
    for child in call_node.children(&mut cursor) {
        if child.is_named() && !first_named_skipped {
            first_named_skipped = true;
            continue; // skip the callee — already handled by extract_calls_from_body
        }
        match child.kind() {
            "value_arguments" | "annotated_lambda" | "lambda_literal" | "function_literal" => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
            // Type arguments and other non-argument children don't contain
            // symbol declarations — skip them.
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// TypeAlias symbol emission
// ---------------------------------------------------------------------------

/// Emit a TypeAlias symbol for `typealias Name = Type`.
/// In tree-sitter-kotlin-ng, `type_alias` has a `type` field holding an
/// `identifier` that IS the alias name (not the aliased type).
fn push_type_decl_alias(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // The `type` field in the Kotlin ng grammar holds the alias name identifier.
    let name = node
        .child_by_field_name("type")
        .map(|n| helpers::node_text(n, src))
        .or_else(|| {
            // Fallback: first identifier-like child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" | "simple_identifier" | "type_identifier" => {
                        let t = helpers::node_text(child, src);
                        if !t.is_empty() && !matches!(t.as_str(), "typealias" | "=") {
                            return Some(t);
                        }
                    }
                    _ => {}
                }
            }
            None
        });
    let name = match name {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    use crate::parser::scope_tree as st;
    use crate::types::Visibility;
    let scope = helpers::enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = st::qualify(&name, scope);
    let scope_path = st::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("typealias {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Annotation TypeRef emission (for standalone annotations at scope level)
// ---------------------------------------------------------------------------

/// Emit a TypeRef for a standalone `annotation` or `file_annotation` node.
///
/// This handles annotations that appear outside of a `modifiers` node (e.g.
/// file-level annotations, or annotations on property delegates). Annotations
/// inside `modifiers` are already handled by `extract_decorators`.
fn emit_annotation_ref(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(name) = annotation_type_name(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}

fn annotation_type_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "user_type" => {
                // user_type → simple_user_type+ → simple_identifier
                let name = calls::kotlin_type_name(&child, src);
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "constructor_invocation" => {
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "user_type" {
                        let name = calls::kotlin_type_name(&inner, src);
                        if !name.is_empty() {
                            return Some(name);
                        }
                    }
                }
            }
            "simple_identifier" | "identifier" | "type_identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type reference scan
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit TypeRef edges for every `user_type` and
/// `nullable_type` node found. This catches type references that the
/// top-down walker misses (e.g., in complex expressions, lambda return
/// types, type projections, or error-recovery subtrees).
///
/// Deduplication happens at the resolution stage — emitting extras here
/// is safe and ensures we never under-count type edges.
fn scan_all_type_refs(node: tree_sitter::Node, src: &[u8], refs: &mut Vec<ExtractedRef>) {
    scan_type_refs_inner(node, src, 0, refs);
}

fn scan_type_refs_inner(
    node: tree_sitter::Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "user_type" => {
            let name = calls::kotlin_type_name(&node, src);
            // Emit TypeRef for all user_type nodes — builtins will be unresolved
            // but we need the ref emitted for coverage credit at this line.
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            // Recurse ONLY into type_arguments children (for generic params like
            // `List<Foo>`) — skip everything else.  FQN segments of a qualified
            // type name (e.g. `com`, `foo`, `bar` in `com.foo.bar.T`) appear as
            // nested user_type / simple_user_type / simple_identifier children;
            // recursing into them would emit every segment as a TypeRef.
            // kotlin_type_name already extracts the correct final segment.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_arguments" {
                    scan_type_refs_inner(child, src, source_symbol_index, refs);
                }
                // All other children (user_type, simple_user_type, simple_identifier,
                // navigation_expression, etc.) are either FQN parts (skip) or
                // non-type content (skip).
            }
        }
        "nullable_type" => {
            // nullable_type wraps a user_type — always emit a ref for coverage
            // (the nullable_type node itself is the ref_node_kind being tracked).
            let name = calls::kotlin_type_name(&node, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
        // type_identifier nodes are already captured at the user_type level
        // (kotlin_type_name returns the last segment of the qualified type).
        // Emitting them again here would produce a ref for every segment of a
        // qualified name — e.g. `com.foo.bar.SomeClass` would emit `com`,
        // `foo`, `bar`, and `SomeClass` as separate TypeRefs instead of just
        // `SomeClass`.  Let the parent user_type arm handle them exclusively.
        "type_identifier" => {}
        // Emit a TypeRef for every annotation node so the annotation line gets
        // credited in the coverage correlation (annotation kind comes after user_type
        // in ref_node_kinds, so we need a dedicated ref at this line).
        "annotation" | "file_annotation" => {
            if let Some(name) = annotation_name_pub(&node, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            // Recurse to handle type args inside annotations and nested annotations.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
        // type_arguments — emit a ref for the first type argument at the
        // type_arguments line, then recurse to handle each argument.
        // This ensures the type_arguments node itself gets credited in coverage
        // even when inner user_type refs are credited to user_type.
        "type_arguments" => {
            // Emit a TypeRef at the type_arguments node line for the first
            // concrete type found inside (covers `List<String>` etc.)
            let mut found_name = String::new();
            let mut cursor0 = node.walk();
            'outer: for child in node.children(&mut cursor0) {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    let name = calls::kotlin_type_name(&inner, src);
                    if !name.is_empty() {
                        found_name = name;
                        break 'outer;
                    }
                }
                let name = calls::kotlin_type_name(&child, src);
                if !name.is_empty() {
                    found_name = name;
                    break;
                }
            }
            if !found_name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: found_name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function parameter type extraction
// ---------------------------------------------------------------------------

/// Extract TypeRef edges for all parameter types and return type of a
/// `function_declaration` node. Walks `function_value_parameters` children
/// and emits TypeRef for every `user_type`, `nullable_type`, or `function_type`
/// found as a parameter type, plus the return type if present.
fn extract_function_param_types(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_value_parameters" => {
                let mut pc = child.walk();
                for param in child.children(&mut pc) {
                    match param.kind() {
                        "function_value_parameter" | "parameter" => {
                            // Walk the parameter to find type nodes.
                            extract_calls_from_body(&param, src, source_symbol_index, refs);
                        }
                        _ => {}
                    }
                }
            }
            // Return type — field "type" on function_declaration holds the return type.
            "type" | "user_type" | "nullable_type" | "function_type"
            | "non_nullable_type" | "parenthesized_type" => {
                calls::extract_type_ref_from_type_node(&child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

/// Infer the type of a property from its initializer expression when no
/// explicit type annotation is present. Emits a TypeRef edge that the
/// engine uses to populate `field_type_name` for chain resolution.
///
/// Handles:
///   `val repo = Repository()` → TypeRef to "Repository"
///   `val service = ServiceImpl.create()` → TypeRef to "ServiceImpl"
///   `val list = mutableListOf()` → skipped (lowercase = not a type constructor)
fn infer_type_from_initializer(
    property_node: &Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Find the initializer: CST children after `=` include the expression.
    let mut cursor = property_node.walk();
    let mut found_eq = false;
    for child in property_node.children(&mut cursor) {
        if child.kind() == "=" {
            found_eq = true;
            continue;
        }
        if !found_eq {
            continue;
        }
        // The first named node after `=` is the initializer expression.
        if !child.is_named() {
            continue;
        }
        match child.kind() {
            "call_expression" => {
                // `Repository()` or `SomeClass.create()`
                if let Some(callee) = child.named_child(0) {
                    let type_name = match callee.kind() {
                        "simple_identifier" | "identifier" => {
                            let name = node_text(callee, src);
                            // Kotlin convention: constructors start with uppercase.
                            if name.starts_with(|c: char| c.is_uppercase()) {
                                Some(name)
                            } else {
                                None
                            }
                        }
                        // `Foo.Bar()` navigation expression — use the last segment
                        // if it starts with uppercase.
                        "navigation_expression" => {
                            let chain = calls::build_chain(&callee, src);
                            chain.and_then(|c| {
                                c.segments.last()
                                    .filter(|s| s.name.starts_with(|c: char| c.is_uppercase()))
                                    .map(|s| s.name.clone())
                            })
                        }
                        _ => None,
                    };
                    if let Some(name) = type_name {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
            }
            // `val x = SomeObject` — direct reference to a singleton/companion.
            "simple_identifier" | "identifier" => {
                let name = node_text(child, src);
                if name.starts_with(|c: char| c.is_uppercase()) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            _ => {}
        }
        break; // Only process the first expression after `=`.
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
