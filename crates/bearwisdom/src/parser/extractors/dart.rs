// =============================================================================
// parser/extractors/dart.rs  —  Dart symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class, Mixin (→ Class), Extension (→ Interface), Enum, EnumMember,
//   Function (top-level), Method, Constructor (named + default),
//   Property (fields), Variable (top-level)
//
// REFERENCES:
//   - `import` / `part` directives → Imports edges
//   - `extends` / `implements` / `with` → Inherits / Implements / TypeRef edges
//   - Method/function body call expressions → Calls edges
//
// Approach:
//   Single-pass recursive CST walk.  Qualified names are threaded through
//   recursion via a `qualified_prefix` string, mirroring python.rs.
//
// Dart grammar node kinds (tree-sitter-dart 0.1):
//   class_definition, mixin_declaration, extension_declaration,
//   enum_declaration, function_signature, method_signature,
//   function_body, import_or_export, part_directive,
//   initialized_variable_definition, field_declaration
// =============================================================================

use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
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

    super::ExtractionResult::new(symbols, refs, has_errors)
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
            // ---- Type declarations ----------------------------------------
            // tree-sitter-dart 0.1 uses `class_declaration` (not `class_definition`)
            "class_declaration" | "class_definition" => {
                extract_class(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "mixin_declaration" => {
                extract_mixin(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "extension_declaration" => {
                extract_extension(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "enum_declaration" => {
                extract_enum(&child, src, symbols, parent_index, qualified_prefix);
            }

            // ---- Top-level functions ----------------------------------------
            "function_signature" | "function_declaration" => {
                if parent_index.is_none() {
                    extract_top_level_function(
                        &child, src, symbols, parent_index, qualified_prefix,
                    );
                }
            }

            // ---- Import / part directives -----------------------------------
            "import_or_export" | "library_import" => {
                extract_import_directive(&child, src, symbols.len(), refs);
            }
            "part_directive" | "part_of_directive" => {
                extract_part_directive(&child, src, symbols.len(), refs);
            }

            // ---- Top-level variable declarations ---------------------------
            "initialized_variable_definition" | "static_final_declaration" => {
                if parent_index.is_none() {
                    extract_variable(&child, src, symbols, parent_index, qualified_prefix);
                }
            }

            "ERROR" | "MISSING" => {}

            _ => {
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Class
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();
    let new_prefix = qualified_name.clone();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("class {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    extract_dart_heritage(node, src, idx, refs);

    if let Some(body) = node.child_by_field_name("body") {
        extract_class_body(&body, src, symbols, refs, Some(idx), &new_prefix);
    }
}

fn extract_class_body(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_signature" | "function_signature" => {
                extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "constructor_signature" => {
                extract_constructor(&child, src, symbols, parent_index, qualified_prefix);
            }
            "field_declaration" | "initialized_variable_definition" => {
                extract_field(&child, src, symbols, parent_index, qualified_prefix);
            }
            "static_final_declaration_list" | "declaration" => {
                extract_class_body(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            _ => {
                extract_class_body(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Mixin
// ---------------------------------------------------------------------------

fn extract_mixin(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();
    let new_prefix = qualified_name.clone();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("mixin {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_class_body(&body, src, symbols, refs, Some(idx), &new_prefix);
    }
}

// ---------------------------------------------------------------------------
// Extension
// ---------------------------------------------------------------------------

fn extract_extension(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
        .unwrap_or_else(|| "<extension>".to_string());
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();
    let new_prefix = qualified_name.clone();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("extension {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_class_body(&body, src, symbols, refs, Some(idx), &new_prefix);
    }
}

// ---------------------------------------------------------------------------
// Enum
// ---------------------------------------------------------------------------

fn extract_enum(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };
    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Enum constants
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_constant" {
            let member_name = get_field_text(&child, src, "name")
                .or_else(|| first_child_text_of_kind(&child, src, "identifier"))
                .unwrap_or_default();

            if member_name.is_empty() {
                continue;
            }

            symbols.push(ExtractedSymbol {
                name: member_name.clone(),
                qualified_name: format!("{qualified_name}.{member_name}"),
                kind: SymbolKind::EnumMember,
                visibility: Some(Visibility::Public),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: Some(qualified_name.clone()),
                parent_index: Some(idx),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Methods / constructors / fields
// ---------------------------------------------------------------------------

fn extract_method(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    let idx = symbols.len();

    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract calls from any immediately following function_body sibling
    if let Some(body) = node.next_sibling() {
        if body.kind() == "function_body" || body.kind() == "block" {
            extract_dart_calls(&body, src, idx, refs);
        }
    }
}

fn extract_constructor(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let raw_name = node_text(*node, src);
    let ctor_name = raw_name
        .find('(')
        .map(|i| raw_name[..i].trim().to_string())
        .unwrap_or_else(|| raw_name.trim().to_string());
    let simple = ctor_name
        .rsplit('.')
        .next()
        .unwrap_or(&ctor_name)
        .to_string();

    let qualified_name = qualify(&simple, qualified_prefix);
    let visibility = if simple.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    symbols.push(ExtractedSymbol {
        name: simple,
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(ctor_name),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

fn extract_field(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // field_declaration wraps initialized_identifier list nodes.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "initialized_identifier" || kind == "identifier" {
            let name = if kind == "initialized_identifier" {
                get_field_text(&child, src, "name")
                    .or_else(|| first_child_text_of_kind(&child, src, "identifier"))
                    .unwrap_or_default()
            } else {
                node_text(child, src)
            };

            if name.is_empty() || name == "final" || name == "static" || name == "late" {
                continue;
            }

            let qualified_name = qualify(&name, qualified_prefix);
            let visibility = if name.starts_with('_') {
                Some(Visibility::Private)
            } else {
                Some(Visibility::Public)
            };

            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: SymbolKind::Property,
                visibility,
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
        }
    }
}

fn extract_top_level_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name}()")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

fn extract_variable(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name = match get_field_text(node, src, "name")
        .or_else(|| first_child_text_of_kind(node, src, "identifier"))
    {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import / part directives
// ---------------------------------------------------------------------------

fn extract_import_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Recursively descend through import_or_export → library_import →
    // import_specification until we find a node that contains the URI.
    extract_import_spec_recursive(node, src, current_symbol_count, refs);
}

fn extract_import_spec_recursive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let k = node.kind();
    // If this node directly contains a URI-like child, extract from it.
    if k == "import_specification" || k == "library_import" || k == "import_or_export" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            if ck == "string_literal" || ck == "uri" || ck == "configurable_uri" {
                let raw = if ck == "configurable_uri" {
                    first_child_text_of_kind(&child, src, "string_literal")
                        .unwrap_or_else(|| node_text(child, src))
                } else {
                    node_text(child, src)
                };
                let module = raw.trim_matches('"').trim_matches('\'').to_string();
                let target = module
                    .rsplit('/')
                    .next()
                    .unwrap_or(&module)
                    .trim_end_matches(".dart")
                    .to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(module),
                    chain: None,
                });
            } else if ck == "import_specification" || ck == "library_import" {
                // Recurse into nested wrappers
                extract_import_spec_recursive(&child, src, current_symbol_count, refs);
            }
        }
    }
}

fn extract_import_spec(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let k = child.kind();
        // tree-sitter-dart 0.1 uses `configurable_uri`; older parsers used
        // `string_literal` or `uri`.
        if k == "string_literal" || k == "uri" || k == "configurable_uri" {
            // The actual URI string may be inside a string_literal child
            let raw = if k == "configurable_uri" {
                // configurable_uri > string_literal > "'"..."'"
                first_child_text_of_kind(&child, src, "string_literal")
                    .unwrap_or_else(|| node_text(child, src))
            } else {
                node_text(child, src)
            };
            let module = raw.trim_matches('"').trim_matches('\'').to_string();
            let target = module
                .rsplit('/')
                .next()
                .unwrap_or(&module)
                .trim_end_matches(".dart")
                .to_string();
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(module),
                chain: None,
            });
        }
    }
}

fn extract_part_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_literal" || child.kind() == "uri" {
            let raw = node_text(child, src);
            let module = raw.trim_matches('"').trim_matches('\'').to_string();
            let target = module
                .rsplit('/')
                .next()
                .unwrap_or(&module)
                .trim_end_matches(".dart")
                .to_string();
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(module),
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Heritage (extends / implements / with)
// ---------------------------------------------------------------------------

fn extract_dart_heritage(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "superclass" => {
                let mut c = child.walk();
                for n in child.children(&mut c) {
                    if n.kind() == "type_name" || n.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::Inherits,
                            line: n.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "interfaces" => {
                let mut c = child.walk();
                for n in child.children(&mut c) {
                    if n.kind() == "type_name" || n.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::Implements,
                            line: n.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "mixins" | "mixin_application" => {
                let mut c = child.walk();
                for n in child.children(&mut c) {
                    if n.kind() == "type_name" || n.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(n, src),
                            kind: EdgeKind::TypeRef,
                            line: n.start_position().row as u32,
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

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_dart_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "invocation_expression" || child.kind() == "function_invocation" {
            let callee_node_opt = child
                .child_by_field_name("function")
                .or_else(|| child.child_by_field_name("name"));

            if let Some(callee_node) = callee_node_opt {
                let chain = build_chain(callee_node, src);

                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| dart_callee_name(callee_node, src));

                if !target_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }
        }
        extract_dart_calls(&child, src, source_symbol_index, refs);
    }
}

fn dart_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "selector_expression" | "navigation_expression" => {
            // Try `selector` field; fall back to the last identifier-like child.
            if let Some(sel) = node.child_by_field_name("selector") {
                return node_text(sel, src);
            }
            let mut last = String::new();
            let mut c = node.walk();
            for n in node.children(&mut c) {
                if n.kind() == "identifier" || n.kind() == "simple_identifier" {
                    last = node_text(n, src);
                }
            }
            last
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

/// Build a structured member-access chain from a Dart invocation callee node.
///
/// Returns `None` for bare single-segment identifiers.
///
/// Dart tree-sitter node shapes:
///   `identifier`           — leaf name
///   `this`                 — receiver keyword
///   `super`                — super keyword
///   `selector_expression`  — `target.selector` member access
///   `navigation_expression`— alternative navigation form (some grammar versions)
///   `cascade_expression`   — `obj..method()` cascade — treat object as receiver
///   `invocation_expression`— nested invocation (chained call)
fn build_chain(node: Node, src: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" | "simple_identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "super" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "selector_expression" => {
            // `object.selector` — named fields vary by grammar version.
            // Try `object` field for the receiver, `selector` for the member.
            let receiver = node
                .child_by_field_name("object")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;

            // The selector may be under a `selector` field or as a direct identifier child.
            let member_name = node
                .child_by_field_name("selector")
                .map(|n| node_text(n, src))
                .or_else(|| {
                    // Scan for the last identifier-like child after the `.`
                    let mut last: Option<String> = None;
                    let mut c = node.walk();
                    for child in node.children(&mut c) {
                        if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                            last = Some(node_text(child, src));
                        }
                    }
                    last
                })?;

            segments.push(ChainSegment {
                name: member_name,
                node_kind: "selector_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "navigation_expression" => {
            // Alternative navigation form — same structure as selector_expression.
            let receiver = node
                .child_by_field_name("target")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;

            let mut last: Option<String> = None;
            let mut c = node.walk();
            for child in node.children(&mut c) {
                if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                    last = Some(node_text(child, src));
                }
            }
            let member_name = last?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "navigation_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "cascade_expression" => {
            // `obj..method()` — the receiver is the first named child before `..`.
            // Just recurse into the first named child (the object).
            let receiver = node.named_child(0)?;
            build_chain_inner(receiver, src, segments)
        }

        "invocation_expression" | "function_invocation" => {
            // Nested invocation in a chain — walk into the function/name child.
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.named_child(0))?;
            build_chain_inner(callee, src, segments)
        }

        // Unknown — can't build a chain.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

fn get_field_text(node: &Node, src: &str, field: &str) -> Option<String> {
    node.child_by_field_name(field).map(|n| node_text(n, src))
}

/// Return the text of the first child whose kind matches `kind`.
/// Uses a `for` loop so that each node's borrow is released before the next
/// iteration — avoiding the cursor lifetime issue with `.find()`.
fn first_child_text_of_kind(node: &Node, src: &str, kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(child, src));
        }
    }
    None
}

fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "dart_tests.rs"]
mod tests;
