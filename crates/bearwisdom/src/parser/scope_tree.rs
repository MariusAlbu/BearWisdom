// =============================================================================
// parser/scope_tree.rs  —  language-agnostic scope tree builder
//
// What is a scope tree?
// ---------------------
// Source code has nested scopes: a namespace contains classes, a class
// contains methods, a method contains local functions.  When we extract a
// symbol we want its *fully qualified name* — the entire chain of enclosing
// scopes joined with dots.
//
// This module builds that chain by doing a depth-first walk over the
// tree-sitter Concrete Syntax Tree (CST).  The walk is driven by a
// `ScopeConfig` that tells it which node kinds "open" a new scope level and
// which field name holds the scope's name.
//
// Example (C#):
//   config.scope_kinds = [
//     "namespace_declaration" → field "name",
//     "class_declaration"     → field "name",
//     "method_declaration"    → field "name",
//   ]
//   Source: `namespace Foo { class Bar { void Baz() {} } }`
//   Scopes built:
//     ScopeNode { name: "Foo",  qualified_name: "Foo",          depth: 0 }
//     ScopeNode { name: "Bar",  qualified_name: "Foo.Bar",      depth: 1 }
//     ScopeNode { name: "Baz",  qualified_name: "Foo.Bar.Baz",  depth: 2 }
//
// Usage by extractors:
//   let tree = scope_tree::build(root_node, source, &CSHARP_SCOPE_CONFIG);
//   let scope = scope_tree::find_scope_at(&tree, byte_offset);
//   let qname = scope.map(|s| s.qualified_name.as_str()).unwrap_or("");
// =============================================================================

use tree_sitter::Node;

/// A single scope entry in the scope tree.
#[derive(Debug, Clone)]
pub struct ScopeEntry {
    /// Simple name of this scope level (e.g. "Bar").
    pub name: String,
    /// Full dotted qualified name up to and including this entry.
    pub qualified_name: String,
    /// The tree-sitter node kind that opened this scope.
    pub node_kind: &'static str,
    /// 0-based byte offset where the scope node starts.
    pub start_byte: usize,
    /// 0-based byte offset where the scope node ends.
    pub end_byte: usize,
    /// Depth in the tree (root scope = 0).
    pub depth: usize,
}

/// Configuration for a single scope-opening node kind.
#[derive(Debug, Clone, Copy)]
pub struct ScopeKind {
    /// The tree-sitter node kind string (e.g. "class_declaration").
    pub node_kind: &'static str,
    /// The field name on that node that holds the scope's name (e.g. "name").
    pub name_field: &'static str,
}

/// A flat list of all scopes found in a file, in DFS order.
///
/// "Flat" means we don't store a tree of pointers.  Instead, each scope
/// records its byte range.  To find the scope that contains a given byte
/// offset you call `find_scope_at` which scans the list and picks the
/// deepest (most specific) scope whose range covers the offset.
pub type ScopeTree = Vec<ScopeEntry>;

/// Build the scope tree for a single source file.
///
/// Parameters:
///   `root`   — the tree-sitter root node (from `tree.root_node()`).
///   `source` — the original source text as bytes (UTF-8).
///   `config` — which node kinds open scopes.
pub fn build(root: Node, source: &[u8], config: &[ScopeKind]) -> ScopeTree {
    let mut tree = Vec::new();
    // Walk from the root with no parent scope yet.
    walk(root, source, config, &[], &mut tree, 0);
    tree
}

/// Find the deepest scope entry that contains `byte_offset`.
///
/// Returns `None` if no scope covers the offset (e.g. top-level code before
/// any namespace declaration).
pub fn find_scope_at(tree: &ScopeTree, byte_offset: usize) -> Option<&ScopeEntry> {
    // Scopes are in DFS order — deeper scopes appear after their parents.
    // We want the deepest (last matching) scope.
    tree.iter()
        .filter(|s| s.start_byte <= byte_offset && byte_offset < s.end_byte)
        .last()
}

/// Find the deepest scope that ENCLOSES `[node_start, node_end)` — i.e. the
/// scope that this node was declared inside, not the scope the node itself
/// opens.
///
/// This excludes any scope entry whose byte range exactly matches
/// `[node_start, node_end)` since that entry was created BY the current node.
///
/// Use this when computing the qualified name for a symbol so that the symbol
/// name is not double-counted in the chain.
pub fn find_enclosing_scope(
    tree: &ScopeTree,
    node_start: usize,
    node_end: usize,
) -> Option<&ScopeEntry> {
    tree.iter()
        .filter(|s| {
            // Scope covers the node's start byte (node is inside the scope).
            s.start_byte <= node_start
                && node_start < s.end_byte
                // Exclude the scope entry that corresponds to the node itself.
                && !(s.start_byte == node_start && s.end_byte == node_end)
        })
        .last()
}

/// Build the qualified name for a symbol given the innermost enclosing scope.
///
/// `symbol_name`: the simple name of the symbol (e.g. "MapCatalogApiV1").
/// `containing_scope`: the result of `find_scope_at` for that symbol's position.
pub fn qualify(symbol_name: &str, containing_scope: Option<&ScopeEntry>) -> String {
    match containing_scope {
        Some(scope) => format!("{}.{symbol_name}", scope.qualified_name),
        None => symbol_name.to_string(),
    }
}

/// Get the scope_path string (parent scope chain) for a symbol position.
///
/// This is everything ABOVE the symbol — i.e. the containing scope's
/// `qualified_name`, or `None` if the symbol is at the top level.
pub fn scope_path(containing_scope: Option<&ScopeEntry>) -> Option<String> {
    containing_scope.map(|s| s.qualified_name.clone())
}

/// Apply a file-level package prefix to top-level symbols.
///
/// Languages with file-scope package declarations (Scala `package foo.bar`,
/// Kotlin `package foo.bar`) put every top-level symbol into that package,
/// but tree-sitter exposes the package_clause / package_header AST node as a
/// sibling of the type declarations, not an ancestor — so the scope-tree
/// walker never sees the package as an enclosing scope. The recommended flow:
///
/// 1. Hoist the package name from the AST.
/// 2. Prepend `<pkg>.` to every `ScopeEntry::qualified_name` in the scope
///    tree. This fixes nested-class qnames automatically (their qname is
///    derived from `qualify(name, scope_entry)`).
/// 3. Run extraction.
/// 4. Call this function with the hoisted package — it walks the symbol
///    list, finds top-level entries (symbols where `scope_path` is None),
///    and rewrites their `qualified_name` and `scope_path` to include the
///    package. It also rewrites descendant qnames whose old prefix matches,
///    catching case-class params and similar children whose qname was
///    stamped during extraction by reading the parent's pre-fixup qname.
///
/// Skips package-clause-emitted Namespace symbols (detected via dotted qname
/// or matching a hoisted-pkg path segment), and brace-form package symbols
/// (qname already contains a dot).
pub fn prefix_top_level_qnames(
    symbols: &mut [crate::types::ExtractedSymbol],
    hoisted_pkg: Option<&str>,
) {
    use crate::types::SymbolKind;

    let n = symbols.len();
    if n == 0 {
        return;
    }

    let mut rewrites: Vec<(usize, String, String)> = Vec::new();

    for i in 0..n {
        if symbols[i].scope_path.is_some() {
            continue;
        }
        if symbols[i].kind == SymbolKind::Namespace {
            if symbols[i].qualified_name.contains('.') {
                continue;
            }
            if let Some(pkg) = hoisted_pkg {
                if pkg.split('.').any(|seg| seg == symbols[i].name) {
                    continue;
                }
            }
        }

        let prefix: Option<String> = match symbols[i].parent_index {
            Some(p_idx) if symbols[p_idx].kind == SymbolKind::Namespace => {
                Some(symbols[p_idx].qualified_name.clone())
            }
            _ => hoisted_pkg.map(|s| s.to_string()),
        };

        let Some(pfx) = prefix else { continue };

        let old = symbols[i].qualified_name.clone();
        let new = format!("{pfx}.{old}");
        rewrites.push((i, old, new));
    }

    for (i, old, new) in rewrites {
        symbols[i].qualified_name = new.clone();
        if symbols[i].scope_path.is_none() {
            let new_scope = new.rsplit_once('.').map(|(p, _)| p.to_string());
            symbols[i].scope_path = new_scope;
        }
        let old_dot = format!("{old}.");
        for j in 0..n {
            if j == i {
                continue;
            }
            if symbols[j].qualified_name.starts_with(&old_dot) {
                let suffix = &symbols[j].qualified_name[old.len()..];
                symbols[j].qualified_name = format!("{new}{suffix}");
            }
            if let Some(sp) = symbols[j].scope_path.clone() {
                if sp == old {
                    symbols[j].scope_path = Some(new.clone());
                } else if sp.starts_with(&old_dot) {
                    let suffix = &sp[old.len()..];
                    symbols[j].scope_path = Some(format!("{new}{suffix}"));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal DFS walker
// ---------------------------------------------------------------------------

fn walk(
    node: Node,
    source: &[u8],
    config: &[ScopeKind],
    parent_chain: &[String], // qualified_name components of all ancestor scopes
    tree: &mut ScopeTree,
    depth: usize,
) {
    // Check if this node opens a new scope.
    if let Some(scope_kind) = config.iter().find(|k| k.node_kind == node.kind()) {
        // Extract the name from the designated field.
        if let Some(name_node) = node.child_by_field_name(scope_kind.name_field) {
            let raw_name = node_text(name_node, source);
            // For C# `qualified_name` nodes (namespace "Foo.Bar"), keep the full text.
            let name = raw_name.clone();

            // Build the full qualified name.
            let qualified_name = if parent_chain.is_empty() {
                name.clone()
            } else {
                format!("{}.{name}", parent_chain.join("."))
            };

            tree.push(ScopeEntry {
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                node_kind: scope_kind.node_kind,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                depth,
            });

            // Build the new parent chain for children.
            // For dotted namespace names ("Foo.Bar") we need to split so
            // children inherit all parts.
            let mut new_chain = parent_chain.to_vec();
            // Push each dot-segment of the name individually so that
            // `qualify("Baz", scope)` gives "Foo.Bar.Baz" not "Foo.Bar.Bar.Baz".
            for part in name.split('.') {
                new_chain.push(part.to_string());
            }

            // Recurse into children with the new parent chain.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk(child, source, config, &new_chain, tree, depth + 1);
            }
            return;
        }
    }

    // Not a scope-creating node — recurse normally without changing the chain.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, config, parent_chain, tree, depth);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, source: &[u8]) -> String {
    // tree-sitter gives us byte ranges; the source is guaranteed UTF-8.
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "scope_tree_tests.rs"]
mod tests;
