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
    // Entries are in DFS pre-order, so start_byte is non-decreasing. Scopes that
    // cover one offset nest, so within the `start_byte <= offset` prefix the
    // deepest (DFS-last) covering scope is the last one whose end_byte > offset.
    // partition_point bounds the scan to that prefix and rev().find stops at the
    // first cover — the same element the old filter(...).last() returned, without
    // touching entries that start after the offset.
    let prefix = tree.partition_point(|s| s.start_byte <= byte_offset);
    tree[..prefix].iter().rev().find(|s| byte_offset < s.end_byte)
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
    // Same DFS-pre-order bound as find_scope_at; additionally exclude the scope
    // entry the node itself opened (an exact [node_start, node_end) match).
    let prefix = tree.partition_point(|s| s.start_byte <= node_start);
    tree[..prefix].iter().rev().find(|s| {
        node_start < s.end_byte && !(s.start_byte == node_start && s.end_byte == node_end)
    })
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
        if matches!(symbols[i].kind, SymbolKind::Namespace | SymbolKind::Module) {
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
            Some(p_idx)
                if matches!(
                    symbols[p_idx].kind,
                    SymbolKind::Namespace | SymbolKind::Module
                ) =>
            {
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
    root: Node,
    source: &[u8],
    config: &[ScopeKind],
    root_chain: &[String], // qualified_name components of all ancestor scopes
    tree: &mut ScopeTree,
    root_depth: usize,
) {
    use std::rc::Rc;

    // Explicit-stack pre-order DFS. A recursive walk overflows on
    // pathologically deep CSTs — a generated `.d.ts` whose single
    // `type X = 'a' | 'b' | …` union nests `union_type` tens of thousands
    // of levels deep is enough to blow the thread stack. The parent chain
    // is shared via `Rc`, so the common non-scope descent clones only a
    // pointer; a fresh chain is built only when a scope node is entered.
    //
    // Children are pushed in reverse so they pop in source order, preserving
    // the DFS pre-order that `find_scope_at` / `find_enclosing_scope` rely on
    // (both pick the *last* matching entry as the deepest scope).
    let mut stack: Vec<(Node, Rc<Vec<String>>, usize)> =
        vec![(root, Rc::new(root_chain.to_vec()), root_depth)];

    while let Some((node, chain, depth)) = stack.pop() {
        // A scope-opening node with a readable name field extends the chain
        // and bumps depth for its children; everything else passes both
        // through unchanged (including a configured scope node missing its
        // name field — mirrors the original fall-through).
        let scope_match = config
            .iter()
            .find(|k| k.node_kind == node.kind())
            .and_then(|k| node.child_by_field_name(k.name_field).map(|n| (k.node_kind, n)));

        let (child_chain, child_depth) = if let Some((node_kind, name_node)) = scope_match {
            // For C# `qualified_name` nodes (namespace "Foo.Bar"), keep the full text.
            let name = node_text(name_node, source);

            let qualified_name = if chain.is_empty() {
                name.clone()
            } else {
                format!("{}.{name}", chain.join("."))
            };

            tree.push(ScopeEntry {
                name: name.clone(),
                qualified_name,
                node_kind,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                depth,
            });

            // Build the new parent chain for children. For dotted namespace
            // names ("Foo.Bar") push each dot-segment individually so that
            // `qualify("Baz", scope)` gives "Foo.Bar.Baz" not "Foo.Bar.Bar.Baz".
            let mut new_chain = (*chain).clone();
            for part in name.split('.') {
                new_chain.push(part.to_string());
            }
            (Rc::new(new_chain), depth + 1)
        } else {
            (chain, depth)
        };

        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push((child, Rc::clone(&child_chain), child_depth));
        }
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
