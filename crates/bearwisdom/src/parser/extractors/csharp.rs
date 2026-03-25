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
    DbMappingSource, EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol,
    SymbolKind, Visibility,
};
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
                    extract_node(body, src, scope_tree, symbols, refs, routes, db_sets, idx);
                }
            }

            "class_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Class);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                // Check if this looks like a DbContext subclass.
                // We'll flag it and scan properties for DbSet<T> below.
                let is_db_context = is_dbcontext_subclass(&child, src);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, routes, db_sets, idx);
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
                    extract_node(body, src, scope_tree, symbols, refs, routes, db_sets, idx);
                }
            }

            "struct_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Struct);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, routes, db_sets, idx);
                }
            }

            "interface_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Interface);
                extract_base_types(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, routes, db_sets, idx);
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
                    // Extract calls from the method body.
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                        // Look for minimal-API route registrations inside the body.
                        extract_minimal_api_routes(&body, src, sym_idx, routes);
                    }
                    // Look for ASP.NET attribute routes on the method declaration.
                    extract_attribute_routes(&child, src, sym_idx, routes);
                }
            }

            "constructor_declaration" => {
                let idx = push_constructor_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Extract type refs from parameter types.
                    push_constructor_type_refs(&child, src, sym_idx, refs);
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
                    let name = callee_name(callee, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                        });
                    }
                }
                // Recurse into arguments (there may be nested calls).
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
// HTTP Route extraction
// ---------------------------------------------------------------------------

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

/// Minimal-API route registration inside method bodies:
///   `app.MapGet("/api/items", ...)` etc.
fn extract_minimal_api_routes(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "invocation_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                if func_node.kind() == "member_access_expression" {
                    if let Some(method_name_node) = func_node.child_by_field_name("name") {
                        let method_name = node_text(method_name_node, src);
                        if let Some(http_method) = http_method_from_attribute(&method_name) {
                            // Extract the first string argument as the route.
                            // In tree-sitter-c-sharp, the argument list field on
                            // invocation_expression is "arguments" (not "argument_list").
                            if let Some(arg_list) = child.child_by_field_name("arguments") {
                                if let Some(template) = first_string_arg(&arg_list, src) {
                                    routes.push(ExtractedRoute {
                                        handler_symbol_index,
                                        http_method: http_method.to_string(),
                                        template,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        // Recurse into block bodies.
        extract_minimal_api_routes(&child, src, handler_symbol_index, routes);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    fn sym(source: &str) -> Vec<ExtractedSymbol> { extract(source).symbols }
    fn refs(source: &str) -> Vec<ExtractedRef>    { extract(source).refs }

    #[test]
    fn extracts_class_with_namespace() {
        let src = "namespace App { public class UserService {} }";
        let symbols = sym(src);
        let svc = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(svc.kind, SymbolKind::Class);
        assert_eq!(svc.visibility, Some(Visibility::Public));
        assert_eq!(svc.qualified_name, "App.UserService");
    }

    #[test]
    fn extracts_interface() {
        let src = "public interface IRepo { void Save(); }";
        let symbols = sym(src);
        let iface = symbols.iter().find(|s| s.name == "IRepo").unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
    }

    #[test]
    fn extracts_enum_and_members() {
        let src = "public enum Color { Red, Green, Blue }";
        let symbols = sym(src);
        assert!(symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum));
        assert!(symbols.iter().any(|s| s.name == "Red" && s.kind == SymbolKind::EnumMember));
        assert!(symbols.iter().any(|s| s.name == "Blue" && s.kind == SymbolKind::EnumMember));
    }

    #[test]
    fn extracts_method_signature() {
        let src = r#"
namespace Catalog {
    class CatalogService {
        public async Task<Item> GetItem(int id) { return null; }
    }
}"#;
        let symbols = sym(src);
        let m = symbols.iter().find(|s| s.name == "GetItem").unwrap();
        assert_eq!(m.kind, SymbolKind::Method);
        assert!(m.signature.as_ref().unwrap().contains("GetItem"));
        assert_eq!(m.qualified_name, "Catalog.CatalogService.GetItem");
    }

    #[test]
    fn extracts_constructor() {
        let src = "class Svc { public Svc(string name) {} }";
        let symbols = sym(src);
        let c = symbols.iter().find(|s| s.kind == SymbolKind::Constructor).unwrap();
        assert_eq!(c.name, "Svc");
    }

    #[test]
    fn extracts_property() {
        let src = "class Foo { public string Name { get; set; } }";
        let symbols = sym(src);
        let p = symbols.iter().find(|s| s.name == "Name").unwrap();
        assert_eq!(p.kind, SymbolKind::Property);
    }

    #[test]
    fn extracts_inheritance_edges() {
        let src = "class Foo : Bar, IBaz {}";
        let r = refs(src);
        assert!(r.iter().any(|r| r.target_name == "Bar" && r.kind == EdgeKind::Inherits));
        assert!(r.iter().any(|r| r.target_name == "IBaz" && r.kind == EdgeKind::Implements));
    }

    #[test]
    fn extracts_call_edges() {
        let src = r#"class S { void Run() { Foo(); bar.Baz(); } }"#;
        let r = refs(src);
        let calls: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let names: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Foo"), "Missing Foo: {names:?}");
        assert!(names.contains(&"Baz"), "Missing Baz: {names:?}");
    }

    #[test]
    fn extracts_instantiation_edges() {
        let src = "class S { void Run() { var x = new Foo(); } }";
        let r = refs(src);
        assert!(r.iter().any(|r| r.target_name == "Foo" && r.kind == EdgeKind::Instantiates));
    }

    #[test]
    fn extracts_http_get_attribute() {
        let src = r#"
class CatalogController {
    [HttpGet("/api/catalog/{id}")]
    public IResult GetById(int id) { return Results.Ok(); }
}"#;
        let result = extract(src);
        assert!(!result.routes.is_empty(), "No routes extracted");
        let route = &result.routes[0];
        assert_eq!(route.http_method, "GET");
        assert!(route.template.contains("catalog"), "Template: {}", route.template);
    }

    #[test]
    fn extracts_test_method_kind() {
        let src = r#"
class Tests {
    [Fact]
    public void ShouldWork() {}
}"#;
        let symbols = sym(src);
        let t = symbols.iter().find(|s| s.name == "ShouldWork").unwrap();
        assert_eq!(t.kind, SymbolKind::Test);
    }

    #[test]
    fn extracts_dbset_properties() {
        let src = r#"
class CatalogDbContext : DbContext {
    public DbSet<CatalogItem> CatalogItems { get; set; }
    public DbSet<CatalogBrand> CatalogBrands { get; set; }
}"#;
        let result = extract(src);
        assert!(!result.db_sets.is_empty(), "No DbSets extracted");
        assert!(result.db_sets.iter().any(|d| d.entity_type == "CatalogItem"));
        assert!(result.db_sets.iter().any(|d| d.entity_type == "CatalogBrand"));
    }

    #[test]
    fn does_not_panic_on_malformed_source() {
        let src = "public class { broken !!! @@@ ###";
        let _ = extract(src); // must not panic
    }

    #[test]
    fn extracts_type_refs_from_method_signature() {
        let src = r#"
using FamilyBudget.Api.Entities;

namespace FamilyBudget.Api.Controllers;

class CategoriesController {
    public async Task<ActionResult<Category>> GetCategories() { return null; }
    public async Task<ActionResult<Category>> CreateCategory(Category category) { return null; }
}
"#;
        let result = extract(src);
        let type_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Category")
            .collect();
        assert!(
            type_refs.len() >= 2,
            "Expected at least 2 Category type refs (return type + parameter), got {}. All refs: {:?}",
            type_refs.len(),
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn extracts_using_as_namespace_import() {
        let src = "using FamilyBudget.Api.Entities;";
        let result = extract(src);
        let imports: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(
            !imports.is_empty(),
            "Expected using directive to produce an Imports ref. Got: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert_eq!(
            imports[0].module.as_deref(),
            Some("FamilyBudget.Api.Entities")
        );
    }

    #[test]
    fn extracts_property_type_ref() {
        let src = r#"
class Transaction {
    public Category? Category { get; set; }
    public int CategoryId { get; set; }
}
"#;
        let result = extract(src);
        let cat_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.target_name == "Category" && r.kind == EdgeKind::TypeRef)
            .collect();
        assert!(
            !cat_refs.is_empty(),
            "Expected Category type ref from property type. Got refs: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn file_scoped_namespace_is_handled() {
        let src = "namespace App.Catalog;\npublic class CatalogApi {}";
        let symbols = sym(src);
        let cls = symbols.iter().find(|s| s.name == "CatalogApi").unwrap();
        assert!(
            cls.qualified_name.contains("CatalogApi"),
            "qualified_name: {}",
            cls.qualified_name
        );
    }

    // WP-6: Record primary constructor parameters extracted as properties.
    #[test]
    fn record_primary_constructor_params_extracted_as_properties() {
        let src = r#"
namespace Geometry {
    public record Point(int X, int Y);
}
"#;
        let symbols = sym(src);
        // The record itself should be extracted as a Class.
        let rec = symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(rec.kind, SymbolKind::Class, "record should be Class kind");

        // X and Y should be extracted as Property symbols with the record as parent.
        let x = symbols
            .iter()
            .find(|s| s.name == "X" && s.kind == SymbolKind::Property);
        assert!(x.is_some(), "Expected property X from record primary ctor");
        let x = x.unwrap();
        assert!(
            x.qualified_name.contains("Point.X"),
            "X.qualified_name should contain 'Point.X', got: {}",
            x.qualified_name
        );
        assert_eq!(
            x.scope_path.as_deref(),
            Some("Geometry.Point"),
            "X.scope_path should be 'Geometry.Point', got: {:?}",
            x.scope_path
        );
        assert_eq!(x.visibility, Some(Visibility::Public));

        let y = symbols.iter().find(|s| s.name == "Y" && s.kind == SymbolKind::Property);
        assert!(y.is_some(), "Expected property Y from record primary ctor");
    }

    // WP-6: Record with body — existing body members not duplicated.
    #[test]
    fn record_with_body_extracts_both_params_and_body_members() {
        let src = r#"
record Person(string Name) {
    public int Age { get; init; }
}
"#;
        let symbols = sym(src);
        let props: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Property).collect();
        // Should have Name (from primary ctor) and Age (from body).
        let names: Vec<&str> = props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Name"), "Expected Name property: {names:?}");
        assert!(names.contains(&"Age"), "Expected Age property: {names:?}");
    }
}
