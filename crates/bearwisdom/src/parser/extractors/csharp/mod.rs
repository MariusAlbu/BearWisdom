// =============================================================================
// parser/extractors/csharp/mod.rs  —  C# symbol and reference extractor
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

mod calls;
mod helpers;
mod symbols;
mod types;

use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{
    EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol, SymbolKind,
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
        if calls::is_csharp_keyword(&r.target_name) {
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
                let idx = symbols::push_namespace(&child, src, scope_tree, symbols, parent_index);
                extract_node(child, src, scope_tree, symbols, refs, routes, db_sets, idx);
            }

            "namespace_declaration" => {
                let idx = symbols::push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "class_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Class);
                types::extract_base_types(&child, src, idx.unwrap_or(0), refs);
                // Extract class-level [Route("...")] for ASP.NET controllers.
                let class_route = calls::extract_class_route_prefix(&child, src);
                // Check if this looks like a DbContext subclass.
                let is_db_context = helpers::is_dbcontext_subclass(&child, src);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, class_route.as_deref());
                    if is_db_context {
                        helpers::extract_db_sets_from_body(&body, src, scope_tree, symbols, db_sets);
                    }
                }
            }

            "record_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Class);
                types::extract_base_types(&child, src, idx.unwrap_or(0), refs);
                // Extract primary constructor parameters as Property symbols.
                // e.g. `record Point(int X, int Y)` → two Property symbols.
                if let Some(record_idx) = idx {
                    symbols::extract_record_primary_params(&child, src, scope_tree, symbols, record_idx);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "struct_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Struct);
                types::extract_base_types(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "interface_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, symbols, parent_index, SymbolKind::Interface);
                types::extract_base_types(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node_inner(body, src, scope_tree, symbols, refs, routes, db_sets, idx, None);
                }
            }

            "enum_declaration" => {
                let idx = symbols::push_enum_decl(&child, src, scope_tree, symbols, parent_index);
                // Enum members are extracted inside push_enum_decl.
                let _ = idx;
            }

            "method_declaration" => {
                let idx = symbols::push_method_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Extract type refs from return type and parameter types.
                    symbols::push_method_type_refs(&child, src, sym_idx, refs);
                    // Extract typed parameters as Property symbols scoped to this method.
                    if let Some(params) = child.child_by_field_name("parameters") {
                        types::extract_csharp_typed_params_as_symbols(params, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    // Extract calls from the method body.
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body(&body, src, sym_idx, refs);
                        // Look for minimal-API route registrations inside the body.
                        calls::extract_minimal_api_routes(&body, src, sym_idx, routes);
                    }
                    // Look for ASP.NET attribute routes on the method declaration.
                    // Prepend the class-level [Route("...")] prefix if present.
                    calls::extract_attribute_routes_with_prefix(&child, src, sym_idx, routes, class_route_prefix);
                }
            }

            "constructor_declaration" => {
                let idx = symbols::push_constructor_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Extract type refs from parameter types.
                    symbols::push_constructor_type_refs(&child, src, sym_idx, refs);
                    // Extract typed parameters as Property symbols scoped to this constructor.
                    if let Some(params) = child.child_by_field_name("parameters") {
                        types::extract_csharp_typed_params_as_symbols(params, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "property_declaration" => {
                symbols::push_property_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "field_declaration" => {
                symbols::push_field_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "event_field_declaration" => {
                symbols::push_event_field_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "delegate_declaration" => {
                symbols::push_delegate_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "using_directive" => {
                symbols::push_using_directive(&child, src, symbols.len(), refs);
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
// Helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "../csharp_tests.rs"]
mod tests;
