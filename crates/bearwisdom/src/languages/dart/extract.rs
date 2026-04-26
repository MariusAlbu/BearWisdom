// =============================================================================
// parser/extractors/dart/mod.rs  —  Dart symbol and reference extractor
// =============================================================================


use super::predicates;
use super::calls::extract_dart_calls;
use super::decorators::{extract_cascade_calls, extract_decorators};
use super::symbols::{
    extract_class, extract_enum, extract_extension, extract_import_directive, extract_mixin,
    extract_part_directive, extract_top_level_function, extract_typedef, extract_variable,
};
use super::helpers::node_text;

use crate::types::{ExtractedRef, ExtractedSymbol, EdgeKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Dart grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, "");

    // Post-traversal: scan the ENTIRE tree for type_identifiers that the
    // walker missed (in nested type arguments, casts, etc.).
    // Use sym_idx=0 as fallback for top-level; most will match a symbol.
    if !symbols.is_empty() {
        scan_all_type_identifiers(tree.root_node(), source, 0, &mut refs);
    }

    // Drop noise refs: type_refs whose target matches a Dart library prefix
    // (the `i1` in `import 'package:foo/foo.dart' as i1`). Generated code
    // (Drift, auto_route, json_serializable, freezed, riverpod_generator)
    // emits qualified types like `i1.AssetFaceEntityCompanion` which the
    // tree-sitter walk records as TWO type_refs — one for the prefix
    // `i1` and one for the type. The prefix is a namespace anchor, never
    // a type, and would never resolve. Skipping it cleanly removes
    // ~1k noise unresolved refs per ts-immich/mobile.
    let aliases = collect_dart_import_aliases(source);
    if !aliases.is_empty() {
        refs.retain(|r| {
            r.kind != EdgeKind::TypeRef || !aliases.contains(&r.target_name)
        });
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

/// Scan a Dart source for `import '<uri>' as <name>` directives,
/// returning the set of `<name>` library prefixes. Used by `extract` to
/// drop `type_ref` refs whose target is just a library prefix rather than
/// a real type.
///
/// Dart's `as <prefix>` only appears in `import` directives — it's the
/// library prefix for qualified references (`import 'foo.dart' as p;
/// p.SomeType`). Exports never take a prefix; `show`/`hide` lists never
/// rename. The regex is therefore safely scoped to imports.
///
/// Robust to multi-line directives:
///
/// ```dart
/// import 'package:foo/foo.dart'
///     as i1
///     show Bar;
/// ```
fn collect_dart_import_aliases(source: &str) -> std::collections::HashSet<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static IMPORT_AS_RE: OnceLock<Regex> = OnceLock::new();
    let re = IMPORT_AS_RE.get_or_init(|| {
        // import ... as <ident> ... ; — `\s` covers newlines for
        // multi-line directives. `(?s)` lets `.` cross newlines.
        Regex::new(r"(?s)\bimport\b[^;]*?\bas\s+([A-Za-z_][A-Za-z0-9_]*)[^;]*?;")
            .expect("static regex compiles")
    });
    let mut out = std::collections::HashSet::new();
    for cap in re.captures_iter(source) {
        if let Some(m) = cap.get(1) {
            out.insert(m.as_str().to_string());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "class_definition" => {
                let pre_len = symbols.len();
                extract_class(&child, src, symbols, refs, parent_index, qualified_prefix);
                // Annotations appear as children of the class_declaration node.
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "mixin_declaration" => {
                let pre_len = symbols.len();
                extract_mixin(&child, src, symbols, refs, parent_index, qualified_prefix);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "extension_declaration" => {
                let pre_len = symbols.len();
                extract_extension(&child, src, symbols, refs, parent_index, qualified_prefix);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "enum_declaration" => {
                let pre_len = symbols.len();
                extract_enum(&child, src, symbols, parent_index, qualified_prefix);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "function_signature" | "function_declaration" => {
                if parent_index.is_none() {
                    let pre_len = symbols.len();
                    extract_top_level_function(&child, src, symbols, parent_index, qualified_prefix);
                    if symbols.len() > pre_len {
                        extract_decorators(&child, src, pre_len, refs);
                        // Extract calls from the sibling function_body (top-level function).
                        let fn_idx = pre_len;
                        if let Some(body) = child.next_sibling() {
                            if body.kind() == "function_body" || body.kind() == "function_expression_body" || body.kind() == "block" {
                                extract_dart_calls(&body, src, fn_idx, refs);
                            }
                        }
                    }
                }
            }
            "import_or_export" | "library_import" | "library_export" => {
                extract_import_directive(&child, src, symbols.len(), refs);
            }
            "part_directive" | "part_of_directive" => {
                extract_part_directive(&child, src, symbols.len(), refs);
            }

            // Dart `typedef` / `type_alias` declarations.
            "type_alias" => {
                extract_typedef(&child, src, symbols, parent_index, qualified_prefix);
            }
            "initialized_variable_definition" | "static_final_declaration" => {
                if parent_index.is_none() {
                    extract_variable(&child, src, symbols, parent_index, qualified_prefix);
                }
            }
            // Cascade expressions at statement level — extract each section's calls.
            "expression_statement" | "return_statement" => {
                if let Some(sym_idx) = parent_index {
                    extract_cascade_calls(&child, src, sym_idx, refs);
                    // Also extract direct invocation expressions in statements.
                    extract_dart_calls(&child, src, sym_idx, refs);
                }
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
            }

            // Direct invocation/function-call expressions outside a statement wrapper.
            "invocation_expression" | "function_invocation" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_dart_calls(&child, src, sym_idx, refs);
            }

            // Function/method body nodes — extract calls within.
            "function_body" | "function_expression_body" | "block" => {
                if let Some(sym_idx) = parent_index {
                    extract_dart_calls(&child, src, sym_idx, refs);
                }
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
            }

            // Catch-all for type_identifier nodes at any recursion level.
            // Emit TypeRef unless it's a Dart builtin.
            "type_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() && !predicates::is_dart_builtin(&name) {
                    if let Some(sym_idx) = parent_index {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
                // type_identifier is a leaf — no children to recurse into.
            }

            // Type-bearing nodes that may contain nested type_identifiers.
            // Scan immediate children for type_identifier AND recurse.
            "type_arguments" | "type_bound" | "function_type" | "type_not_void"
            | "type_not_void_not_function" | "declared_type" => {
                // Scan immediate children for type_identifier to catch generic args.
                let mut tc = child.walk();
                for grandchild in child.children(&mut tc) {
                    if grandchild.kind() == "type_identifier" && grandchild.is_named() {
                        let name = node_text(grandchild, src);
                        if !name.is_empty() && !predicates::is_dart_builtin(&name) {
                            if let Some(idx) = parent_index {
                                refs.push(ExtractedRef {
                                    source_symbol_index: idx,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: grandchild.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                    }
                }
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
            }

            // factory_constructor_signature at top-level visit (e.g. inside class body
            // nodes that bypass extract_class_body).
            "factory_constructor_signature" => {
                extract_factory_constructor_at_visit(&child, src, symbols, refs, parent_index, qualified_prefix);
            }

            "ERROR" | "MISSING" => {}
            _ => {
                // Universal type_identifier scanner — recursively finds ALL
                // type_identifiers in this subtree before normal recursion.
                if let Some(idx) = parent_index {
                    scan_all_type_identifiers(child, src, idx, refs);
                }
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Factory constructor helper for visit() context
// ---------------------------------------------------------------------------

/// Extract a factory constructor symbol when encountered directly in `visit()`.
/// Delegates to the symbols::extract_class_body path by routing through
/// `symbols::extract_factory_constructor_inline`.
fn extract_factory_constructor_at_visit(
    node: &tree_sitter::Node,
    src: &str,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    use super::helpers::{node_text as nt, qualify};
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::scope_from_prefix;

    // Walk children: find the constructor name (type_identifier / qualified_name).
    let mut name: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let t = nt(child, src);
                if !t.is_empty() && t != "factory" {
                    name = Some(t);
                    break;
                }
            }
            "qualified_name" => {
                let mut last: Option<String> = None;
                let mut qc = child.walk();
                for inner in child.children(&mut qc) {
                    if inner.kind() == "identifier" || inner.kind() == "type_identifier" {
                        last = Some(nt(inner, src));
                    }
                }
                if let Some(n) = last {
                    name = Some(n);
                    break;
                }
            }
            _ => {}
        }
    }
    let name = match name {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("factory {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
    // Also emit TypeRef for type_identifier children (return type annotations, params).
    let idx = symbols.len() - 1;
    let mut tc = node.walk();
    for child in node.children(&mut tc) {
        if child.kind() == "type_identifier" && child.is_named() {
            let t = nt(child, src);
            if !t.is_empty() && !predicates::is_dart_builtin(&t) {
                refs.push(ExtractedRef {
                    source_symbol_index: idx,
                    target_name: t,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
    }
}

/// Recursively scan ALL descendants of `node` for `type_identifier` and emit TypeRef.
/// This is the "nuclear option" — ensures no type_identifier is missed regardless of
/// how deeply nested it is (e.g., `Map<String, List<User>>` → finds `User`).
fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    src: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" && child.is_named() {
            let name = node_text(child, src);
            if !name.is_empty() && !predicates::is_dart_builtin(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        // Recurse into ALL children regardless
        scan_all_type_identifiers(child, src, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

