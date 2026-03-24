// =============================================================================
// parser/extractors/rust.rs  —  Rust symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct, Enum, EnumMember, Interface (trait), Method, Function,
//   TypeAlias, Variable (static), Namespace (mod), Test
//
// REFERENCES:
//   - `use` declarations     → Import edges (recursive use-tree walking)
//   - `call_expression`      → Calls edges
//
// Approach:
//   Single-pass recursive CST walk. No scope tree — qualified names are built
//   by threading a `qualified_prefix` string through the recursion. `impl`
//   blocks are not symbols themselves; they set the prefix for their methods.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct RustExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract all symbols and references from Rust source code.
pub fn extract(source: &str) -> RustExtraction {
    let language = tree_sitter_rust::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Rust grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return RustExtraction {
                symbols: vec![],
                refs: vec![],
                has_errors: true,
            }
        }
    };

    let mut symbols = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(
        tree.root_node(),
        source,
        &mut symbols,
        &mut refs,
        None,
        "",
    );

    let has_errors = tree.root_node().has_error();
    RustExtraction { symbols, refs, has_errors }
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            // `function_item` — function with a body.
            // `function_signature_item` — bare fn signature inside a trait (no `{}`).
            "function_item" | "function_signature_item" => {
                if let Some(sym) =
                    extract_function(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, source, idx, refs);
                    }
                }
            }

            "struct_item" => {
                if let Some(sym) =
                    extract_struct(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "enum_item" => {
                if let Some(sym) =
                    extract_enum(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_enum_variants(&body, source, Some(idx), &new_prefix, symbols);
                    }
                }
            }

            "trait_item" => {
                if let Some(sym) =
                    extract_trait(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "impl_item" => {
                extract_impl(&child, source, symbols, refs, qualified_prefix);
            }

            "type_item" => {
                if let Some(sym) =
                    extract_type_alias(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "const_item" => {
                if let Some(sym) =
                    extract_const(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "static_item" => {
                if let Some(sym) =
                    extract_static(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "mod_item" => {
                if let Some(sym) =
                    extract_mod(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "use_declaration" => {
                extract_use_names(&child, source, refs, symbols.len());
            }

            // macro_definition — skip intentionally
            "macro_definition" => {}

            // Skip tree-sitter error recovery nodes
            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// impl block handling
// ---------------------------------------------------------------------------

/// Process an `impl_item` — not a symbol itself, but the container for methods.
/// The implementing type name becomes the qualified prefix for its methods.
fn extract_impl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    outer_prefix: &str,
) {
    // `type` field = the type being implemented (rhs of `impl Trait for Type`
    // or simply `impl Type`).
    let type_node = match node.child_by_field_name("type") {
        Some(n) => n,
        None => return,
    };
    let type_name = node_text(&type_node, source);

    let impl_prefix = if outer_prefix.is_empty() {
        type_name
    } else {
        format!("{outer_prefix}.{type_name}")
    };

    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            if let Some(sym) = extract_method_from_fn(&child, source, None, &impl_prefix) {
                let idx = symbols.len();
                symbols.push(sym);
                if let Some(fn_body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&fn_body, source, idx, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol extractors
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_signature(node, source);

    let kind = if has_test_attribute(node, source) {
        SymbolKind::Test
    } else {
        SymbolKind::Function
    };

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// Same as `extract_function` but always emits `Method` kind (used inside impl blocks).
fn extract_method_from_fn(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_signature(node, source);

    let kind = if has_test_attribute(node, source) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

fn extract_struct(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("struct {name}");
    if let Some(tp) = node.child_by_field_name("type_parameters") {
        sig.push_str(&node_text(&tp, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Struct,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

fn extract_enum(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("enum {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Enum,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// Extract `enum_variant` children from an enum body into the symbol list.
fn extract_enum_variants(
    body: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            // tree-sitter-rust uses `name` field on enum_variant nodes.
            // Fall back to the first named identifier child if the field is missing.
            let field_name_node = child.child_by_field_name("name");
            let name_node = if field_name_node.is_some() {
                field_name_node
            } else {
                let mut variant_cursor = child.walk();
                let found = child
                    .children(&mut variant_cursor)
                    .find(|n| n.is_named() && n.kind() == "identifier");
                found
            };

            if let Some(name_node) = name_node {
                let name = node_text(&name_node, source);
                let qualified_name = qualify(&name, qualified_prefix);
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::EnumMember,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: extract_doc_comment(&child, source),
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
    }
}

fn extract_trait(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("trait {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Interface,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

fn extract_type_alias(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("type {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

fn extract_const(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("const {name}");
    if let Some(ty) = node.child_by_field_name("type") {
        sig.push_str(": ");
        sig.push_str(&node_text(&ty, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        // v3 maps Constant → Variable (no separate Constant kind)
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

fn extract_static(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("static {name}");
    if let Some(ty) = node.child_by_field_name("type") {
        sig.push_str(": ");
        sig.push_str(&node_text(&ty, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

fn extract_mod(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("mod {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

/// Walk a `use_declaration` node and emit `Import` references for every
/// leaf name that is actually imported.
///
/// Examples handled:
///   `use foo::bar::Baz;`         → target "Baz", module "foo::bar"
///   `use foo::bar::{Baz, Qux};`  → "Baz" and "Qux", module "foo::bar"
///   `use foo::bar::*;`           → target "*", module "foo::bar"
///   `use foo::bar as B;`         → target "B", module "foo"
fn extract_use_names(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier"
            | "scoped_use_list"
            | "use_as_clause"
            | "use_wildcard"
            | "identifier"
            | "use_list" => {
                walk_use_tree(&child, source, refs, current_symbol_count, "");
            }
            _ => {}
        }
    }
}

/// Recursively walk the use-tree, accumulating the path prefix and emitting a
/// reference for every leaf name.
fn walk_use_tree(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    prefix: &str,
) {
    match node.kind() {
        // `foo::bar::Baz` — path ending with an identifier
        "scoped_identifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();

            if name.is_empty() {
                return;
            }

            let module = build_module_path(prefix, &path);
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module: if module.is_empty() { None } else { Some(module) },
            });
        }

        // `foo::bar::{Baz, Qux}` — group import
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let new_prefix = build_module_path(prefix, &path);

            if let Some(list) = node.child_by_field_name("list") {
                walk_use_tree(&list, source, refs, current_symbol_count, &new_prefix);
            }
        }

        // `{Baz, Qux, inner::Thing}` — brace list
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "{" | "}" | "," => {}
                    _ => walk_use_tree(&child, source, refs, current_symbol_count, prefix),
                }
            }
        }

        // `Baz as B` — alias: emit the alias name
        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|n| node_text(&n, source));
            let original = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source));

            let target = alias.or(original).unwrap_or_default();
            if target.is_empty() {
                return;
            }

            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };

            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
            });
        }

        // `use foo::*;`
        "use_wildcard" => {
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: "*".to_string(),
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
            });
        }

        // Plain `identifier` — e.g. `use std;`
        "identifier" => {
            let name = node_text(node, source);
            if name.is_empty() {
                return;
            }
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
            });
        }

        _ => {
            // Recurse into anything we don't recognise
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_use_tree(&child, source, refs, current_symbol_count, prefix);
            }
        }
    }
}

fn build_module_path(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => path.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}::{path}"),
    }
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

/// Recursively scan a function/method body for `call_expression` nodes
/// and emit `Calls` references.
fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func) = child.child_by_field_name("function") {
                let callee_text = node_text(&func, source);

                // Extract the simple name:
                //   `Foo::bar`   → "bar"  (path-style call)
                //   `obj.method` → "method" (method call via field_expression)
                //   `foo`        → "foo"
                let simple_name = callee_text
                    .rsplit("::")
                    .next()
                    .unwrap_or(&callee_text)
                    .rsplit('.')
                    .next()
                    .unwrap_or(&callee_text)
                    .trim()
                    .to_string();

                if !simple_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple_name,
                        kind: EdgeKind::Calls,
                        line: func.start_position().row as u32,
                        module: None,
                    });
                }
            }
        }
        extract_calls_from_body(&child, source, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: &Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()].to_string()
}

fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// Build `scope_path` from the qualified prefix: the prefix is already the
/// full ancestor path, so we use it directly.
fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Build a signature from the first source line of the item, stripping the
/// trailing `{` (and any whitespace before it).
fn extract_signature(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    let first_line = text.lines().next()?;
    let sig = first_line.trim_end_matches('{').trim().to_string();
    if sig.is_empty() {
        None
    } else {
        Some(sig)
    }
}

/// Detect the Rust visibility of an item by inspecting the `visibility_modifier`
/// child node.
///
/// | Source text       | Result                       |
/// |-------------------|------------------------------|
/// | `pub`             | `Some(Visibility::Public)`   |
/// | `pub(crate)`      | `Some(Visibility::Internal)` |
/// | `pub(super)`      | `Some(Visibility::Protected)`|
/// | `pub(in path)`    | `Some(Visibility::Internal)` |
/// | _(no modifier)_   | `Some(Visibility::Private)`  |
fn detect_visibility(node: &Node) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let mut has_restriction = false;
            let mut is_super = false;
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "crate" | "self" | "in" => has_restriction = true,
                    "super" => {
                        has_restriction = true;
                        is_super = true;
                    }
                    "identifier" | "scoped_identifier" => has_restriction = true,
                    _ => {}
                }
            }

            return Some(if !has_restriction {
                Visibility::Public
            } else if is_super {
                Visibility::Protected
            } else {
                Visibility::Internal
            });
        }
    }
    // No visibility modifier — Rust default is private
    Some(Visibility::Private)
}

/// Collect consecutive `///` or `//!` doc-comment lines immediately preceding
/// this node (as previous siblings). Also handles `/** ... */` block doc
/// comments. Returns the combined text, or `None` if there are no doc comments.
fn extract_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();

    let mut current = node.prev_sibling();
    while let Some(sib) = current {
        match sib.kind() {
            "line_comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("///") || text.starts_with("//!") {
                    lines.push(text);
                    current = sib.prev_sibling();
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("/**") {
                    lines.push(text);
                }
                break;
            }
            _ => break,
        }
    }

    if lines.is_empty() {
        return None;
    }

    lines.reverse();
    Some(lines.join("\n"))
}

/// Return `true` if the `function_item` has an `attribute_item` sibling
/// (immediately preceding, possibly separated by other attribute items or
/// comments) that contains `test`.
///
/// Matches `#[test]`, `#[tokio::test]`, `#[async_std::test]`, etc.
fn has_test_attribute(node: &Node, source: &str) -> bool {
    let mut current = node.prev_sibling();
    while let Some(sib) = current {
        match sib.kind() {
            "attribute_item" => {
                let text = node_text(&sib, source);
                if text.contains("test") {
                    return true;
                }
                current = sib.prev_sibling();
            }
            "line_comment" | "block_comment" => {
                current = sib.prev_sibling();
            }
            _ => break,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impl_method_qualified_name() {
        let source = r#"struct Bar;

impl Bar {
    pub fn foo(&self) {}
}"#;
        let r = extract(source);
        let method = r.symbols.iter().find(|s| s.name == "foo");
        assert!(method.is_some(), "Expected method 'foo'");
        assert_eq!(method.unwrap().qualified_name, "Bar.foo");
        assert_eq!(method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn use_declaration_produces_import_ref() {
        let source = "use crate::db::Database;";
        let r = extract(source);
        let import_refs: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(!import_refs.is_empty(), "Expected at least one import ref");
        let names: Vec<&str> = import_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(
            names.contains(&"Database"),
            "Expected 'Database' in import targets, got: {names:?}"
        );
        let db_ref = import_refs
            .iter()
            .find(|r| r.target_name == "Database")
            .unwrap();
        assert_eq!(
            db_ref.module.as_deref(),
            Some("crate::db"),
            "Expected module 'crate::db'"
        );
    }

    #[test]
    fn enum_produces_enum_and_members() {
        let source = r#"enum Foo {
    A,
    B,
}"#;
        let r = extract(source);
        let enum_sym = r.symbols.iter().find(|s| s.name == "Foo");
        assert!(enum_sym.is_some(), "Expected 'Foo' enum");
        assert_eq!(enum_sym.unwrap().kind, SymbolKind::Enum);

        let members: Vec<_> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::EnumMember)
            .collect();
        assert_eq!(members.len(), 2, "Expected 2 enum members, got {}", members.len());
        let names: Vec<&str> = members.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"A"), "Missing 'A'");
        assert!(names.contains(&"B"), "Missing 'B'");
    }

    #[test]
    fn trait_maps_to_interface_kind() {
        let source = "pub trait MyTrait { fn do_it(&self); }";
        let r = extract(source);
        let trait_sym = r.symbols.iter().find(|s| s.name == "MyTrait");
        assert!(trait_sym.is_some(), "Expected 'MyTrait'");
        assert_eq!(trait_sym.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn mod_maps_to_namespace_kind() {
        let source = r#"mod inner {
    pub fn foo() {}
}"#;
        let r = extract(source);
        let mod_sym = r.symbols.iter().find(|s| s.name == "inner");
        assert!(mod_sym.is_some(), "Expected 'inner' mod");
        assert_eq!(mod_sym.unwrap().kind, SymbolKind::Namespace);
        let fn_sym = r.symbols.iter().find(|s| s.name == "foo");
        assert_eq!(fn_sym.unwrap().qualified_name, "inner.foo");
    }

    #[test]
    fn extracts_pub_function() {
        let source = r#"pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}"#;
        let r = extract(source);
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.symbols[0].name, "greet");
        assert_eq!(r.symbols[0].kind, SymbolKind::Function);
        assert_eq!(r.symbols[0].visibility, Some(Visibility::Public));
    }

    #[test]
    fn extracts_use_group_imports() {
        let source = "use std::collections::{HashMap, HashSet};";
        let r = extract(source);
        let names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(names.contains(&"HashMap"), "Missing HashMap: {names:?}");
        assert!(names.contains(&"HashSet"), "Missing HashSet: {names:?}");
    }

    #[test]
    fn extracts_test_function() {
        let source = r#"#[test]
fn test_something() {
    assert_eq!(1, 1);
}"#;
        let r = extract(source);
        let test_sym = r.symbols.iter().find(|s| s.name == "test_something");
        assert!(test_sym.is_some());
        assert_eq!(test_sym.unwrap().kind, SymbolKind::Test);
    }

    #[test]
    fn extracts_call_references() {
        let source = r#"fn run() {
    foo();
    bar.baz();
}"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(call_names.contains(&"foo"), "Missing 'foo': {call_names:?}");
        assert!(call_names.contains(&"baz"), "Missing 'baz': {call_names:?}");
    }

    #[test]
    fn attaches_doc_comment() {
        let source = r#"/// Documentation for foo.
pub fn foo() {}"#;
        let r = extract(source);
        assert_eq!(r.symbols.len(), 1);
        let doc = r.symbols[0].doc_comment.as_deref().unwrap_or("");
        assert!(doc.contains("Documentation for foo"), "Got: {doc:?}");
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = r#"fn broken( { let x = ;"#;
        let r = extract(source);
        // Must not panic; partial results are acceptable.
        let _ = r.symbols;
    }
}
