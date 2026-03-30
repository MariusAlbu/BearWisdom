// =============================================================================
// parser/extractors/csharp.rs  —  C# symbol and reference extractor
//
// This is the primary extractor — eShop is a C# project.
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace, Class, Struct, Interface, Record, Enum, EnumMember,
//   Method, Constructor, Property, Field, Event, Delegate
//
// REFERENCES (used to build edges):
//   - `using` directives        → Import record (for resolution priority 3)
//   - `: BaseClass, IInterface` → Inherits / Implements edges
//   - `invocation_expression`   → Calls edges
//   - `object_creation_expression` → Instantiates edges
//
// ROUTES (for HTTP connector):
//   - `[HttpGet("...")]`, `[HttpPost("...")]` etc. on methods
//   - `[Route("...")]` on controllers / methods
//   - `app.MapGet(...)`, `app.MapPost(...)` minimal-API calls
//
// DB SETS (for EF Core connector):
//   - `DbSet<T>` properties on DbContext subclasses
//   - `[Table("name")]` attribute on entity classes
//
// Approach
// --------
// 1. First pass: build a scope tree so we know the qualified name of every
//    position in the file (see scope_tree.rs).
// 2. Second pass: walk the CST extracting symbols, inserting them with the
//    qualified name derived from the scope tree.
// 3. Reference extraction happens inside the second pass — every method body
//    is scanned for calls and every type declaration for base-type lists.
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{
    ChainSegment, DbMappingSource, EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute,
    ExtractedSymbol, MemberChain, SegmentKind, SymbolKind, Visibility,
};
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration for C#
// ---------------------------------------------------------------------------

/// These are the node kinds that create a new scope level in C#.
/// `name_field` is the tree-sitter field name that holds the simple name.
static CSHARP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_declaration",            name_field: "name" },
    ScopeKind { node_kind: "file_scoped_namespace_declaration", name_field: "name" },
    ScopeKind { node_kind: "class_declaration",                 name_field: "name" },
    ScopeKind { node_kind: "struct_declaration",                name_field: "name" },
    ScopeKind { node_kind: "interface_declaration",             name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",                  name_field: "name" },
    ScopeKind { node_kind: "record_declaration",                name_field: "name" },
    ScopeKind { node_kind: "method_declaration",                name_field: "name" },
    ScopeKind { node_kind: "constructor_declaration",           name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// The complete result of extracting one C# file.
pub struct CSharpExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub routes: Vec<ExtractedRoute>,
    pub db_sets: Vec<ExtractedDbSet>,
    pub has_errors: bool,
}

/// Parse `source` and extract all symbols, references, routes, and DbSet mappings.
///
/// Returns `has_errors = true` if tree-sitter found syntax errors, but extraction
/// proceeds anyway (partial results are better than none for large codebases).
pub fn extract(source: &str) -> CSharpExtraction {
    // --- Set up tree-sitter parser ---
    let language: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load C# grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return CSharpExtraction {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
                has_errors: true,
            }
        }
    };

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();
    let root = tree.root_node();

    // --- Build the scope tree (first pass) ---
    // This gives us a flat list of all scope entries with their byte ranges
    // and qualified names.  We'll use it to look up the scope of any node.
    let scope_tree = scope_tree::build(root, src_bytes, CSHARP_SCOPE_KINDS);

    // --- Extract symbols and references (second pass) ---
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let mut routes: Vec<ExtractedRoute> = Vec::new();
    let mut db_sets: Vec<ExtractedDbSet> = Vec::new();

    // First pass: extract all symbols and refs (with unqualified call targets).
    extract_node(
        root,
        src_bytes,
        &scope_tree,
        &mut symbols,
        &mut refs,
        &mut routes,
        &mut db_sets,
        None, // no parent symbol yet
    );

    // Collect using directives for qualification context.
    let usings: Vec<String> = refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.module.clone().unwrap_or_else(|| r.target_name.clone()))
        .filter(|m| m.contains('.'))
        .collect();

    // Second pass: qualify unresolved call/instantiates/type_ref targets using scope + usings.
    for r in &mut refs {
        if r.target_name.contains('.') {
            continue; // Already qualified
        }
        if r.kind != EdgeKind::Calls && r.kind != EdgeKind::Instantiates && r.kind != EdgeKind::TypeRef {
            continue;
        }
        if is_csharp_keyword(&r.target_name) {
            continue;
        }

        let ref_kind = r.kind;

        // Try scope chain qualification
        let byte_offset = {
            let target_line = r.line as usize;
            src_bytes.iter().enumerate()
                .filter(|(_, &b)| b == b'\n')
                .nth(target_line.saturating_sub(1))
                .map(|(i, _)| i)
                .unwrap_or(0)
        };

        if let Some(scope) = scope_tree::find_scope_at(&scope_tree, byte_offset) {
            let mut chain = scope.qualified_name.as_str();
            let mut found = false;
            loop {
                let candidate = format!("{chain}.{}", r.target_name);
                if symbols.iter().any(|s| s.qualified_name == candidate && ref_kind_matches_symbol(ref_kind, s.kind)) {
                    r.target_name = candidate;
                    found = true;
                    break;
                }
                match chain.rfind('.') {
                    Some(pos) => chain = &chain[..pos],
                    None => {
                        let candidate = format!("{chain}.{}", r.target_name);
                        if symbols.iter().any(|s| s.qualified_name == candidate && ref_kind_matches_symbol(ref_kind, s.kind)) {
                            r.target_name = candidate;
                            found = true;
                        }
                        break;
                    }
                }
            }
            if found { continue; }
        }

        // Try using directives
        for ns in &usings {
            let candidate = format!("{ns}.{}", r.target_name);
            if symbols.iter().any(|s| s.qualified_name == candidate && ref_kind_matches_symbol(ref_kind, s.kind)) {
                r.target_name = candidate;
                break;
            }
        }
    }

    CSharpExtraction { symbols, refs, routes, db_sets, has_errors }
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn extract_node(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    routes: &mut Vec<ExtractedRoute>,
    db_sets: &mut Vec<ExtractedDbSet>,
    parent_index: Option<usize>,
) {
    extract_node_inner(node, src, scope_tree, symbols, refs, routes, db_sets, parent_index, None);
}

#[allow(clippy::too_many_arguments)]
fn extract_node_inner(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    routes: &mut Vec<ExtractedRoute>,
    db_sets: &mut Vec<ExtractedDbSet>,
    parent_index: Option<usize>,
    class_route_prefix: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // ----------------------------------------------------------------
            // Skip — already handled by scope tree, or irrelevant syntax
            // ----------------------------------------------------------------
            "file_scoped_namespace_declaration" => {
                // Emit a Namespace symbol so same-namespace resolution works
                // for file-scoped declarations like `namespace X.Y.Z;`
                let idx = push_namespace(&child, src, scope_tree, symbols, parent_index);
                extract_node(child, src, scope_tree, symbols, refs, routes, db_sets, idx);
            }

            "namespace_declaration" => {
                let idx = push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "class_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Class);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                // Extract class-level [Route("...")] for ASP.NET controllers.
                let class_route = extract_class_route_prefix(&child, src);
                // Check if this looks like a DbContext subclass.
                let is_db_context = is_dbcontext_subclass(&child, src);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, class_route.as_deref());
                    if is_db_context {
                        extract_db_sets_from_body(&body, src, scope_tree, symbols, db_sets);
                    }
                }
            }

            "record_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Class);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                // Extract primary constructor parameters as Property symbols.
                // e.g. `record Point(int X, int Y)` → two Property symbols.
                if let Some(record_idx) = idx {
                    extract_record_primary_params(&child, src, scope_tree, symbols, record_idx);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "struct_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Struct);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "interface_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Interface);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "enum_declaration" => {
                let idx = push_enum_decl(&child, src, scope_tree, symbols, parent_index);
                // Enum members are extracted inside push_enum_decl.
                let _ = idx;
            }

            "method_declaration" => {
                let idx = push_method_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Extract type refs from return type and parameter types.
                    push_method_type_refs(&child, src, sym_idx, refs);
                    // Extract typed parameters as Property symbols scoped to this method.
                    if let Some(params) = child.child_by_field_name("parameters") {
                        extract_csharp_typed_params_as_symbols(params, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    // Extract calls from the method body.
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                        // Look for minimal-API route registrations inside the body.
                        extract_minimal_api_routes(&body, src, sym_idx, routes);
                    }
                    // Look for ASP.NET attribute routes on the method declaration.
                    // Prepend the class-level [Route("...")] prefix if present.
                    extract_attribute_routes_with_prefix(&child, src, sym_idx, routes, class_route_prefix);
                }
            }

            "constructor_declaration" => {
                let idx = push_constructor_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Extract type refs from parameter types.
                    push_constructor_type_refs(&child, src, sym_idx, refs);
                    // Extract typed parameters as Property symbols scoped to this constructor.
                    if let Some(params) = child.child_by_field_name("parameters") {
                        extract_csharp_typed_params_as_symbols(params, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "property_declaration" => {
                push_property_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "field_declaration" => {
                push_field_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "event_field_declaration" => {
                push_event_field_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "delegate_declaration" => {
                push_delegate_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "using_directive" => {
                push_using_directive(&child, src, symbols.len(), refs);
            }

            "ERROR" | "MISSING" => {
                // tree-sitter error recovery nodes — skip but don't crash.
            }

            _ => {
                // Recurse into any container we don't explicitly handle.
                extract_node(child, src, scope_tree, symbols, refs, routes, db_sets, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers (one per declaration kind)
// ---------------------------------------------------------------------------

fn push_namespace(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);
    // Use parent scope (byte before this node), not the namespace's own scope.
    // Same pattern as push_type_decl — prevents doubled names like "App.App".
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_type_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    kind: SymbolKind,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    // The scope tree has an entry for this class at this byte position.
    // We find the scope CONTAINING this class (its parent), not the class scope itself.
    // The class's own scope entry has start_byte == node.start_byte().
    // `find_scope_at` returns the deepest scope covering the start byte —
    // which will be the class itself if depth > 0.
    // We want the parent scope, so we look up the position just *before* this node.
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let keyword = match kind {
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        _ => "class",
    };
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{keyword} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract primary constructor parameters of a record as Property symbols.
///
/// `record Point(int X, int Y)` — `X` and `Y` are synthesised as public
/// init-only properties by the compiler.  We extract them so the index
/// knows they exist (they won't appear in a body as `property_declaration`).
fn extract_record_primary_params(
    record_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    record_sym_idx: usize,
) {
    // The record's own scope covers its parameter list and body.
    // We find the record's qualified name from the symbol we just pushed.
    let record_qname = symbols[record_sym_idx].qualified_name.clone();

    // In tree-sitter-c-sharp, `parameter_list` is an unnamed child of
    // `record_declaration`, not a named field.  Use find_child_kind.
    let param_list = match find_child_kind(record_node, "parameter_list") {
        Some(pl) => pl,
        None => return, // record without a primary constructor parameter list
    };

    let mut cursor = param_list.walk();
    for param in param_list.children(&mut cursor) {
        if param.kind() != "parameter" {
            continue;
        }
        let name_node = match param.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(name_node, src);
        let type_str = param
            .child_by_field_name("type")
            .map(|t| node_text(t, src))
            .unwrap_or_default();

        let qualified_name = format!("{record_qname}.{name}");
        // Use the record's own scope entry as the parent scope.
        let parent_scope = scope_tree::find_scope_at(scope_tree, param.start_byte());
        let scope_path = Some(record_qname.clone());
        let _ = parent_scope; // scope lookup not needed — we derive scope_path directly

        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: Some(crate::types::Visibility::Public),
            start_line: param.start_position().row as u32,
            end_line: param.end_position().row as u32,
            start_col: param.start_position().column as u32,
            end_col: param.end_position().column as u32,
            signature: Some(format!("{type_str} {name}")),
            doc_comment: None,
            scope_path,
            parent_index: Some(record_sym_idx),
        });
    }
}

fn push_enum_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
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
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Enum,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Extract enum members.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            if member.kind() == "enum_member_declaration" {
                if let Some(mname_node) = member.child_by_field_name("name") {
                    let mname = node_text(mname_node, src);
                    let mqualified = format!("{qualified_name}.{mname}");
                    symbols.push(ExtractedSymbol {
                        name: mname,
                        qualified_name: mqualified,
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: member.start_position().row as u32,
                        end_line: member.end_position().row as u32,
                        start_col: member.start_position().column as u32,
                        end_col: member.end_position().column as u32,
                        signature: None,
                        doc_comment: extract_doc_comment(&member, src),
                        scope_path: Some(qualified_name.clone()),
                        parent_index: Some(idx),
                    });
                }
            }
        }
    }

    Some(idx)
}

fn push_method_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    // The method's own scope covers its body — we want the parent (the class).
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let kind = if has_test_attribute(node, src) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: build_method_signature(node, src),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract type refs from a method's return type and parameter types.
/// Called after the symbol is pushed so we know its index.
fn push_method_type_refs(
    node: &Node,
    src: &[u8],
    symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Return type is the `returns` field on method_declaration.
    if let Some(ret_node) = node.child_by_field_name("returns") {
        extract_type_refs_from_type_node(ret_node, src, symbol_index, refs);
    }
    // Parameter types.
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, symbol_index, refs);
    }
}

fn push_constructor_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
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
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract type refs from a constructor's parameter types.
fn push_constructor_type_refs(
    node: &Node,
    src: &[u8],
    symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, symbol_index, refs);
    }
}

fn push_property_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
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

    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{type_str} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit a TypeRef edge for the property's declared type.
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_from_type_node(type_node, src, idx, refs);
    }
}

fn push_field_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let is_const = has_modifier(node, "const");
    let kind = if is_const { SymbolKind::Field } else { SymbolKind::Field };
    let visibility = detect_visibility(node, src);
    let doc_comment = extract_doc_comment(node, src);

    let var_decl = match find_child_kind(node, "variable_declaration") {
        Some(v) => v,
        None => return,
    };
    let type_str = var_decl
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    // Grab the type node once; we'll emit a TypeRef per field declarator.
    let type_node_opt = var_decl.child_by_field_name("type");

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    let mut cursor = var_decl.walk();
    for declarator in var_decl.children(&mut cursor) {
        if declarator.kind() == "variable_declarator" {
            if let Some(name_node) = declarator.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = scope_tree::qualify(&name, parent_scope);
                let idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name,
                    kind,
                    visibility,
                    start_line: declarator.start_position().row as u32,
                    end_line: declarator.end_position().row as u32,
                    start_col: declarator.start_position().column as u32,
                    end_col: declarator.end_position().column as u32,
                    signature: Some(format!("{type_str} {name}")),
                    doc_comment: doc_comment.clone(),
                    scope_path: scope_path.clone(),
                    parent_index,
                });
                // Emit a TypeRef for the field's declared type.
                if let Some(tn) = type_node_opt {
                    extract_type_refs_from_type_node(tn, src, idx, refs);
                }
            }
        }
    }
}

fn push_event_field_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let visibility = detect_visibility(node, src);
    let doc_comment = extract_doc_comment(node, src);

    let var_decl = match find_child_kind(node, "variable_declaration") {
        Some(v) => v,
        None => return,
    };
    let type_str = var_decl
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    let mut cursor = var_decl.walk();
    for declarator in var_decl.children(&mut cursor) {
        if declarator.kind() == "variable_declarator" {
            if let Some(name_node) = declarator.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = scope_tree::qualify(&name, parent_scope);
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name,
                    kind: SymbolKind::Event,
                    visibility,
                    start_line: node.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                    start_col: node.start_position().column as u32,
                    end_col: node.end_position().column as u32,
                    signature: Some(format!("event {type_str} {name}")),
                    doc_comment: doc_comment.clone(),
                    scope_path: scope_path.clone(),
                    parent_index,
                });
            }
        }
    }
}

fn push_delegate_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
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

    let ret = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Delegate,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("delegate {ret} {name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import / using directive
// ---------------------------------------------------------------------------

fn push_using_directive(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Skip `using Alias = ...` — these are type aliases, not namespace imports.
    if node.child_by_field_name("name").is_some() {
        return;
    }

    // Extract the full namespace path from the using directive and emit a
    // single Imports edge whose `module` IS the full namespace.
    // e.g. `using FamilyBudget.Api.Entities;` →
    //   target_name: "FamilyBudget.Api.Entities", module: Some("FamilyBudget.Api.Entities")
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(name),
                    chain: None,
                });
                return;
            }
            "qualified_name" => {
                let full = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: full.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                    chain: None,
                });
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Inheritance / interface implementation
// ---------------------------------------------------------------------------

fn extract_base_types(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "base_list" {
            let mut first_concrete = true;
            let mut cursor = child.walk();
            for base in child.children(&mut cursor) {
                match base.kind() {
                    "identifier" | "generic_name" | "qualified_name" => {
                        let name = simple_type_name(base, src);
                        let kind = if looks_like_interface(&name) {
                            EdgeKind::Implements
                        } else if first_concrete {
                            first_concrete = false;
                            EdgeKind::Inherits
                        } else {
                            EdgeKind::TypeRef
                        };
                        if looks_like_interface(&name) {
                            // Don't flip first_concrete for interfaces.
                        }
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
}

fn looks_like_interface(name: &str) -> bool {
    let mut chars = name.chars();
    matches!((chars.next(), chars.next()), (Some('I'), Some(c)) if c.is_uppercase())
}

fn simple_type_name(node: Node, src: &[u8]) -> String {
    if node.kind() == "generic_name" {
        let children: Vec<Node> = {
            let mut cursor = node.walk();
            node.children(&mut cursor).collect()
        };
        if let Some(id) = children.iter().find(|c| c.kind() == "identifier") {
            return node_text(*id, src);
        }
    }
    if node.kind() == "qualified_name" {
        let full = node_text(node, src);
        return full.rsplit('.').next().unwrap_or(&full).to_string();
    }
    node_text(node, src)
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "invocation_expression" => {
                if let Some(callee) = child.child_by_field_name("function") {
                    let chain = build_chain(callee, src);
                    let name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name(callee, src));
                    if !name.is_empty() && !is_csharp_keyword(&name) {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = simple_type_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Check if a ref kind is compatible with a symbol kind for qualification.
/// TypeRef should match classes/interfaces/enums, Calls should match methods, etc.
fn ref_kind_matches_symbol(ref_kind: EdgeKind, sym_kind: SymbolKind) -> bool {
    match ref_kind {
        EdgeKind::Calls => matches!(sym_kind, SymbolKind::Method | SymbolKind::Function | SymbolKind::Constructor),
        EdgeKind::TypeRef => matches!(sym_kind, SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Enum | SymbolKind::Namespace),
        EdgeKind::Instantiates => matches!(sym_kind, SymbolKind::Class | SymbolKind::Struct),
        EdgeKind::Inherits => matches!(sym_kind, SymbolKind::Class | SymbolKind::Struct),
        EdgeKind::Implements => matches!(sym_kind, SymbolKind::Interface),
        _ => true,
    }
}

/// C# keywords/operators that look like method calls but aren't.
fn is_csharp_keyword(name: &str) -> bool {
    matches!(
        name,
        "nameof" | "typeof" | "sizeof" | "default" | "checked" | "unchecked"
        | "stackalloc" | "await" | "throw" | "yield" | "var" | "is" | "as"
        | "new" | "this" | "base" | "null" | "true" | "false" | "value"
    )
}

fn callee_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_access_expression" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, src))
            .unwrap_or_else(|| {
                let t = node_text(node, src);
                t.rsplit('.').next().unwrap_or(&t).to_string()
            }),
        "generic_name" => {
            // Generic method call like `GetService<T>()` — extract just the name.
            let children: Vec<Node> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).collect()
            };
            children
                .iter()
                .find(|c| c.kind() == "identifier")
                .map(|n| node_text(*n, src))
                .unwrap_or_default()
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// MemberChain building
// ---------------------------------------------------------------------------

/// Build a structured member access chain from tree-sitter AST nodes.
///
/// Recursively walks nested `member_access_expression` nodes to produce
/// a `Vec<ChainSegment>` from root to leaf.
///
/// `this.repo.FindOne()` tree structure:
/// ```text
/// invocation_expression
///   function: member_access_expression
///     expression: member_access_expression
///       expression: this_expression "this"
///       name: identifier "repo"
///     name: identifier "FindOne"
/// ```
/// produces: `[this, repo, FindOne]`
fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "this_expression" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "base_expression" => {
            segments.push(ChainSegment {
                name: "base".to_string(),
                node_kind: "base_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "generic_name" => {
            // `GetService<T>` — strip the generic args, keep just the identifier.
            let name = {
                let mut cursor = node.walk();
                let children: Vec<Node> = node.children(&mut cursor).collect();
                drop(cursor);
                children
                    .iter()
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(*c, src))
                    .unwrap_or_else(|| node_text(node, src))
            };
            segments.push(ChainSegment {
                name,
                node_kind: "generic_name".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_access_expression" => {
            let expr = node.child_by_field_name("expression")?;
            let name_node = node.child_by_field_name("name")?;

            // Recurse into the expression (receiver) to build the prefix chain.
            build_chain_inner(expr, src, segments)?;

            // The name may be a generic_name (e.g., `Foo<T>`) — extract identifier.
            let name = if name_node.kind() == "generic_name" {
                let mut cursor = name_node.walk();
                let children: Vec<Node> = name_node.children(&mut cursor).collect();
                drop(cursor);
                children
                    .iter()
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(*c, src))
                    .unwrap_or_else(|| node_text(name_node, src))
            } else {
                node_text(name_node, src)
            };

            segments.push(ChainSegment {
                name,
                node_kind: name_node.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "conditional_access_expression" => {
            // C# `?.` operator: `foo?.Bar()`
            let expr = node.child_by_field_name("expression")?;
            let binding = node.child_by_field_name("binding")?;

            build_chain_inner(expr, src, segments)?;

            // The binding is a `member_binding_expression` with a `name` field.
            let name_node = binding.child_by_field_name("name").unwrap_or(binding);
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: binding.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: true,
            });
            Some(())
        }

        "invocation_expression" => {
            // Nested call in a chain: `a.B().C()` — the expression is an invocation.
            // Walk into the function child to continue the chain.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // Unknown node — can't build a chain.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// HTTP Route extraction
// ---------------------------------------------------------------------------

/// Extract the class-level `[Route("...")]` attribute value for ASP.NET controllers.
///
/// Example: `[Route("api/categories")]` → `Some("api/categories")`
fn extract_class_route_prefix(class_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if name == "Route" {
                            return attr_route_template(&attr, src);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Attribute-based route extraction with optional class-level prefix.
fn extract_attribute_routes_with_prefix(
    node: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
    class_prefix: Option<&str>,
) {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let attr_name = node_text(name_node, src);
                        if let Some(method) = http_method_from_attribute(&attr_name) {
                            let method_template = attr_route_template(&attr, src)
                                .unwrap_or_else(|| String::from(""));
                            // Combine class prefix with method template.
                            let template = match class_prefix {
                                Some(prefix) if !prefix.is_empty() => {
                                    let p = prefix.trim_matches('/');
                                    let m = method_template.trim_matches('/');
                                    if m.is_empty() {
                                        format!("/{p}")
                                    } else {
                                        format!("/{p}/{m}")
                                    }
                                }
                                _ => {
                                    if method_template.is_empty() {
                                        "/".to_string()
                                    } else {
                                        method_template
                                    }
                                }
                            };
                            routes.push(ExtractedRoute {
                                handler_symbol_index,
                                http_method: method.to_string(),
                                template,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Attribute-based route extraction — reads `[HttpGet("...")]` etc. on methods.
fn extract_attribute_routes(
    node: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
) {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let attr_name = node_text(name_node, src);
                        if let Some(method) = http_method_from_attribute(&attr_name) {
                            // Extract the route template from the attribute argument.
                            let template = attr_route_template(&attr, src)
                                .unwrap_or_else(|| String::from("/"));
                            routes.push(ExtractedRoute {
                                handler_symbol_index,
                                http_method: method.to_string(),
                                template,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn http_method_from_attribute(name: &str) -> Option<&'static str> {
    // Strip generic suffix if present: `HttpGet<T>` → `HttpGet`
    let base = name.split('<').next().unwrap_or(name);
    match base {
        "HttpGet" | "MapGet" => Some("GET"),
        "HttpPost" | "MapPost" => Some("POST"),
        "HttpPut" | "MapPut" => Some("PUT"),
        "HttpDelete" | "MapDelete" => Some("DELETE"),
        "HttpPatch" | "MapPatch" => Some("PATCH"),
        "Route" => Some("ANY"),
        _ => None,
    }
}

fn attr_route_template(attr_node: &Node, src: &[u8]) -> Option<String> {
    // In tree-sitter-c-sharp the attribute argument list is a child NODE of kind
    // `attribute_argument_list` — it is NOT a named field, so child_by_field_name
    // will always return None.  We must find it by kind.
    //
    // Structure:
    //   attribute
    //     identifier              ← name (this IS a named field)
    //     attribute_argument_list ← kind (NOT a named field)
    //       (
    //       attribute_argument
    //         string_literal
    //           string_literal_content  ← raw text, no quotes
    //       )
    let arg_list = find_child_kind(attr_node, "attribute_argument_list")?;
    let mut cursor = arg_list.walk();
    for arg in arg_list.children(&mut cursor) {
        if arg.kind() == "attribute_argument" {
            let mut ac = arg.walk();
            for child in arg.children(&mut ac) {
                match child.kind() {
                    "string_literal" => {
                        // Prefer string_literal_content (the text without surrounding quotes).
                        let children: Vec<Node> = {
                            let mut sc = child.walk();
                            child.children(&mut sc).collect()
                        };
                        if let Some(content) = children.iter().find(|c| c.kind() == "string_literal_content") {
                            return Some(node_text(*content, src));
                        }
                        // Fallback: strip quotes from the whole string_literal text.
                        let raw = node_text(child, src);
                        return Some(raw.trim_matches('"').to_string());
                    }
                    "verbatim_string_literal" => {
                        let raw = node_text(child, src);
                        let stripped = raw.trim_start_matches('@').trim_matches('"');
                        return Some(stripped.to_string());
                    }
                    "interpolated_string_expression" => {
                        return Some("{dynamic}".to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Combine a route prefix with a route template.
///
/// Examples:
///   ("api/auth", "login")       → "api/auth/login"
///   ("api/auth", "/")           → "api/auth"
///   ("", "login")               → "login"
///   ("api/catalog", "{id:int}") → "api/catalog/{id:int}"
fn combine_route_prefix(prefix: &str, action: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let action = action.trim_matches('/');

    if prefix.is_empty() {
        return if action.is_empty() { "/".to_string() } else { action.to_string() };
    }
    if action.is_empty() {
        return prefix.to_string();
    }
    format!("{prefix}/{action}")
}

/// Minimal-API route registration inside method bodies:
///   `app.MapGet("/api/items", ...)` etc.
///
/// Also resolves `MapGroup` prefixes:
///   `var api = app.MapGroup("api/orders"); api.MapGet("/", handler);`
///   → route template becomes `"api/orders"` instead of `"/"`.
fn extract_minimal_api_routes(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
) {
    let group_prefixes = build_mapgroup_prefixes(body, src);
    extract_minimal_api_routes_inner(body, src, handler_symbol_index, routes, &group_prefixes);
}

/// Build a map of variable names to their accumulated MapGroup prefix.
fn build_mapgroup_prefixes<'a>(body: &Node<'a>, src: &[u8]) -> HashMap<String, String> {
    let mut prefixes: HashMap<String, String> = HashMap::new();
    collect_mapgroup_assignments(body, src, &mut prefixes);
    prefixes
}

/// Recursively walk a block collecting `var X = expr.MapGroup("prefix")` assignments.
fn collect_mapgroup_assignments(node: &Node, src: &[u8], prefixes: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "local_declaration_statement"
            || child.kind() == "variable_declaration"
        {
            collect_mapgroup_assignments(&child, src, prefixes);
            continue;
        }

        if child.kind() == "variable_declarator" {
            let var_name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, src));

            // The initializer is a direct child of variable_declarator after `=`.
            let mut found_eq = false;
            let mut init_expr: Option<Node> = None;
            let mut vc = child.walk();
            for vchild in child.children(&mut vc) {
                if vchild.kind() == "=" {
                    found_eq = true;
                } else if found_eq && vchild.kind() == "invocation_expression" {
                    init_expr = Some(vchild);
                    break;
                }
            }

            if let (Some(var_name), Some(init)) = (var_name, init_expr) {
                if let Some(prefix) = resolve_mapgroup_chain(&init, src, prefixes) {
                    prefixes.insert(var_name, prefix);
                }
            }
            continue;
        }

        collect_mapgroup_assignments(&child, src, prefixes);
    }
}

/// Resolve the group prefix from a (possibly chained) expression.
fn resolve_mapgroup_chain(
    node: &Node,
    src: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<String> {
    if node.kind() != "invocation_expression" {
        return None;
    }

    let func_node = node.child_by_field_name("function")?;

    if func_node.kind() == "member_access_expression" {
        let method_name = node_text(func_node.child_by_field_name("name")?, src);
        let object = func_node.child_by_field_name("expression")?;

        if method_name == "MapGroup" {
            let arg_list = node.child_by_field_name("arguments")?;
            let group_path = first_string_arg(&arg_list, src)?;
            let receiver_prefix = resolve_receiver_prefix(&object, src, prefixes);

            return Some(combine_route_prefix(
                &receiver_prefix.unwrap_or_default(),
                &group_path,
            ));
        }

        // Fluent chain: `.HasApiVersion(...)`, etc. — recurse into the object.
        return resolve_mapgroup_chain(&object, src, prefixes);
    }

    None
}

/// Get the accumulated prefix for a receiver expression.
fn resolve_receiver_prefix(
    object: &Node,
    src: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<String> {
    match object.kind() {
        "identifier" => {
            let name = node_text(*object, src);
            prefixes.get(&name).cloned()
        }
        "invocation_expression" => resolve_mapgroup_chain(object, src, prefixes),
        _ => None,
    }
}

/// Get the variable name from the receiver of a member_access_expression.
fn get_receiver_name(func_node: &Node, src: &[u8]) -> Option<String> {
    let object = func_node.child_by_field_name("expression")?;
    if object.kind() == "identifier" {
        Some(node_text(object, src))
    } else {
        None
    }
}

/// Inner recursive route extractor with group prefix support.
fn extract_minimal_api_routes_inner(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
    group_prefixes: &HashMap<String, String>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "invocation_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                if func_node.kind() == "member_access_expression" {
                    if let Some(method_name_node) = func_node.child_by_field_name("name") {
                        let method_name = node_text(method_name_node, src);
                        if let Some(http_method) = http_method_from_attribute(&method_name) {
                            if let Some(arg_list) = child.child_by_field_name("arguments") {
                                if let Some(template) = first_string_arg(&arg_list, src) {
                                    let prefix = get_receiver_name(&func_node, src)
                                        .and_then(|name| group_prefixes.get(&name).cloned())
                                        .unwrap_or_default();

                                    let full_template = if prefix.is_empty() {
                                        template
                                    } else {
                                        combine_route_prefix(&prefix, &template)
                                    };

                                    routes.push(ExtractedRoute {
                                        handler_symbol_index,
                                        http_method: http_method.to_string(),
                                        template: full_template,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        extract_minimal_api_routes_inner(&child, src, handler_symbol_index, routes, group_prefixes);
    }
}

fn first_string_arg(arg_list: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = arg_list.walk();
    for arg in arg_list.children(&mut cursor) {
        if arg.kind() == "argument" {
            let mut ac = arg.walk();
            for child in arg.children(&mut ac) {
                match child.kind() {
                    "string_literal" => {
                        // Prefer the `string_literal_content` child (no surrounding quotes).
                        let children: Vec<Node> = {
                            let mut sc = child.walk();
                            child.children(&mut sc).collect()
                        };
                        if let Some(content) = children.iter().find(|c| c.kind() == "string_literal_content") {
                            return Some(node_text(*content, src));
                        }
                        let raw = node_text(child, src);
                        return Some(raw.trim_matches('"').to_string());
                    }
                    "verbatim_string_literal" => {
                        let raw = node_text(child, src);
                        return Some(raw.trim_start_matches('@').trim_matches('"').to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// EF Core DbSet<T> extraction
// ---------------------------------------------------------------------------

/// Returns true if this class declaration inherits from DbContext (directly or
/// via a named subclass ending in "Context").
fn is_dbcontext_subclass(node: &Node, src: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_list" {
            let mut bc = child.walk();
            for base in child.children(&mut bc) {
                let name = match base.kind() {
                    "identifier" => node_text(base, src),
                    "generic_name" | "qualified_name" => simple_type_name(base, src),
                    _ => continue,
                };
                if name.contains("DbContext") {
                    return true;
                }
            }
        }
    }
    false
}

/// Walk the body of a DbContext class and collect all `DbSet<T>` properties.
fn extract_db_sets_from_body(
    body: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &[ExtractedSymbol],
    db_sets: &mut Vec<ExtractedDbSet>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "property_declaration" {
            // Check if the type is DbSet<T>.
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_str = node_text(type_node, src);
                if type_str.starts_with("DbSet<") {
                    // Extract T from DbSet<T>.
                    let entity_type = type_str
                        .trim_start_matches("DbSet<")
                        .trim_end_matches('>')
                        .trim()
                        .to_string();

                    // Find the symbol for this property in the symbols vec.
                    let name_node = match child.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let prop_name = node_text(name_node, src);

                    // Find the symbol index (linear scan — fine for small DbContext classes).
                    let prop_sym_idx = symbols
                        .iter()
                        .rposition(|s| s.name == prop_name && s.kind == SymbolKind::Property)
                        .unwrap_or(0);

                    // Determine table name: check for [Table("...")] attribute first.
                    // We'll look on the entity class — that's a cross-file concern, but
                    // we record what we can here.  The connector will enrich this later.
                    let table_name = entity_type.clone(); // convention: plural is applied by connector
                    let source = check_table_attribute_on_property(&child, src)
                        .map(|_| DbMappingSource::Attribute)
                        .unwrap_or(DbMappingSource::Convention);

                    let _ = scope_tree; // available for future enrichment

                    db_sets.push(ExtractedDbSet {
                        property_symbol_index: prop_sym_idx,
                        entity_type,
                        table_name,
                        source,
                    });
                }
            }
        }
    }
}

/// Returns the table name from a [Table("...")] attribute if present.
fn check_table_attribute_on_property(node: &Node, src: &[u8]) -> Option<String> {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al = child.walk();
            for attr in child.children(&mut al) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        if node_text(name_node, src) == "Table" {
                            return attr_route_template(&attr, src);
                        }
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

fn detect_visibility(node: &Node, _src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // tree-sitter-c-sharp wraps each modifier keyword in a `modifier` node.
        if child.kind() == "modifier" {
            let mut mc = child.walk();
            for kw in child.children(&mut mc) {
                match kw.kind() {
                    "public" => return Some(Visibility::Public),
                    "private" => return Some(Visibility::Private),
                    "protected" => return Some(Visibility::Protected),
                    "internal" => return Some(Visibility::Internal),
                    _ => {}
                }
            }
        }
    }
    None
}

fn has_modifier(node: &Node, keyword: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
            let mut mc = child.walk();
            if child.children(&mut mc).any(|kw| kw.kind() == keyword) {
                return true;
            }
        }
    }
    false
}

const TEST_ATTRIBUTES: &[&str] = &["Test", "Fact", "Theory", "TestMethod", "TestCase"];

fn has_test_attribute(node: &Node, src: &[u8]) -> bool {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al = child.walk();
            for attr in child.children(&mut al) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if TEST_ATTRIBUTES.contains(&name.as_str()) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Collect consecutive `///` doc-comment siblings immediately before `node`.
fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        if s.kind() == "comment" {
            let text = node_text(s, src);
            if text.trim_start().starts_with("///") {
                lines.push(text);
                sib = s.prev_sibling();
                continue;
            }
        }
        break;
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

fn build_method_signature(node: &Node, src: &[u8]) -> Option<String> {
    let name = node_text(node.child_by_field_name("name")?, src);
    let ret = node
        .child_by_field_name("returns")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    Some(format!("{ret} {name}{type_params}{params}").trim().to_string())
}

fn find_child_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let children: Vec<Node<'a>> = {
        let mut cursor = node.walk();
        node.children(&mut cursor).collect()
    };
    children.into_iter().find(|c| c.kind() == kind)
}

/// Returns true for C# primitive / standard-library type names that are not
/// useful to track as cross-file references.
fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "int"
            | "long"
            | "double"
            | "float"
            | "decimal"
            | "bool"
            | "byte"
            | "char"
            | "short"
            | "uint"
            | "ulong"
            | "ushort"
            | "sbyte"
            | "object"
            | "void"
            | "dynamic"
            | "var"
            | "Task"
            | "IActionResult"
            | "ActionResult"
            | "IResult"
            | "Results"
            | "IEnumerable"
            | "IList"
            | "List"
            | "Dictionary"
            | "HashSet"
            | "ICollection"
            | "IReadOnlyList"
            | "IReadOnlyCollection"
            | "Nullable"
    )
}

/// Walk a type node (which may be a generic, nullable, array, etc.) and emit a
/// `TypeRef` edge for every non-builtin named type found inside it.
///
/// Handles:
/// - `identifier`         — e.g. `Category`
/// - `generic_name`       — e.g. `ActionResult<Category>` → emits `ActionResult` skipped
///                          (builtin) but recurses into type_argument_list → `Category`
/// - `nullable_type`      — e.g. `Category?`
/// - `array_type`         — e.g. `Category[]`
/// - `qualified_name`     — e.g. `Foo.Bar` → simple name `Bar`
/// - `type_argument_list` — children are the generic arguments
fn extract_type_refs_from_type_node(
    type_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match type_node.kind() {
        "identifier" => {
            let name = node_text(type_node, src);
            if !name.is_empty() && !is_builtin_type(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "qualified_name" => {
            let full = node_text(type_node, src);
            let simple = full.rsplit('.').next().unwrap_or(&full).to_string();
            if !simple.is_empty() && !is_builtin_type(&simple) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "generic_name" => {
            // e.g. `ActionResult<Category>` — recurse into the identifier (the
            // outer type name) and the type_argument_list (the inner types).
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => {
                        let name = node_text(child, src);
                        if !name.is_empty() && !is_builtin_type(&name) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    "type_argument_list" => {
                        // Recurse into each type argument.
                        let mut ac = child.walk();
                        for arg in child.children(&mut ac) {
                            extract_type_refs_from_type_node(arg, src, source_symbol_index, refs);
                        }
                    }
                    _ => {}
                }
            }
        }
        "nullable_type" => {
            // e.g. `Category?` — the inner type is the first named child.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                extract_type_refs_from_type_node(child, src, source_symbol_index, refs);
            }
        }
        "array_type" => {
            // e.g. `Category[]` — the element type is a named child.
            if let Some(elem) = type_node.child_by_field_name("type") {
                extract_type_refs_from_type_node(elem, src, source_symbol_index, refs);
            }
        }
        "predefined_type" => {
            // e.g. `int`, `string`, `bool` — always builtins, skip.
        }
        _ => {
            // For any other wrapper node (e.g. `ref_type`, tuple types) recurse
            // into children so we don't miss nested named types.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                extract_type_refs_from_type_node(child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract type refs from all parameters of a parameter_list node.
fn extract_type_refs_from_params(
    params_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter" {
            if let Some(type_node) = child.child_by_field_name("type") {
                extract_type_refs_from_type_node(type_node, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract typed parameters from a C# `parameter_list` node as Property symbols
/// scoped to the enclosing method or constructor.
///
/// For `void Process(UserRepository repo, int id)`, creates:
///   Symbol: `Namespace.Class.Process.repo` (kind=Property)
///   TypeRef: `Namespace.Class.Process.repo → UserRepository`
///
/// Skips parameters with predefined/builtin types and parameters without names.
///
/// C# `parameter` structure:
///   `attribute_list*`, optional `_parameter_type_with_modifiers` (has `type` field),
///   `name` field (identifier), optional default value
fn extract_csharp_typed_params_as_symbols(
    params_node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Find the method/constructor scope — params are qualified under it.
    let method_scope = if params_node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, params_node.start_byte())
    } else {
        None
    };

    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() != "parameter" {
            continue;
        }

        let name_node = match child.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(name_node, src);
        if name.is_empty() {
            continue;
        }

        let type_node = match child.child_by_field_name("type") {
            Some(t) => t,
            None => continue,
        };

        // Skip predefined (builtin) types — they don't reference user symbols.
        if type_node.kind() == "predefined_type" {
            continue;
        }

        // Extract a simple type name for the TypeRef target.
        let type_name = csharp_param_type_name(type_node, src);
        if type_name.is_empty() || is_builtin_type(&type_name) {
            continue;
        }

        let qualified_name = scope_tree::qualify(&name, method_scope);
        let scope_path = scope_tree::scope_path(method_scope);

        let param_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: None,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: Some(format!("{type_name} {name}")),
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Emit a TypeRef from the param symbol to its type.
        extract_type_refs_from_type_node(type_node, src, param_idx, refs);
    }
}

/// Extract a simple display name from a C# type node for use in signatures.
fn csharp_param_type_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "qualified_name" => {
            let full = node_text(node, src);
            full.rsplit('.').next().unwrap_or(&full).to_string()
        }
        "generic_name" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return node_text(child, src);
                }
            }
            String::new()
        }
        "nullable_type" => {
            // `Foo?` — extract inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let name = csharp_param_type_name(child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "array_type" => {
            node.child_by_field_name("type")
                .map(|t| csharp_param_type_name(t, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "csharp_tests.rs"]
mod tests;
