// =============================================================================
// rust/calls_imports.rs  —  `extern crate` and `use` declaration ref extraction
// =============================================================================

use super::helpers::{detect_visibility, node_text, qualify, scope_from_prefix};
use crate::types::{AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// extern crate import
// ---------------------------------------------------------------------------

/// Emit an `Imports` edge for `extern crate foo;`.
///
/// tree-sitter-rust shape:
/// ```text
/// extern_crate_declaration
///   "extern" "crate"
///   name: identifier  "foo"
///   ["as" alias: identifier]
/// ```
pub(super) fn extract_extern_crate(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    if name.is_empty() || name == "self" {
        return;
    }
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: current_symbol_count,
        target_name: name,
        kind: EdgeKind::Imports,
        line: name_node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: name_node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

/// Walk a `use_declaration` node and emit `Import` references for every
/// leaf name that is actually imported.
pub(super) fn extract_use_names(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    current_symbol_count: usize,
    qualified_prefix: &str,
    alias_targets: &mut Vec<(String, AliasTarget)>,
) {
    // A `pub` / `pub(crate)` / `pub(super)` use re-exports the imported names
    // onto the module's surface — those names are genuine re-exports the binder
    // may follow to their definition. A private `use` only brings names into
    // local scope and must NOT enter the re-export map: following it would bind
    // a name through a module that merely imports it (Invariant #2).
    let is_reexport = !matches!(detect_visibility(node), Some(Visibility::Private));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "scoped_use_list" | "use_as_clause" | "use_wildcard"
            | "identifier" | "use_list" => {
                walk_use_tree(
                    &child,
                    source,
                    refs,
                    symbols,
                    current_symbol_count,
                    "",
                    is_reexport,
                    qualified_prefix,
                    alias_targets,
                );
            }
            _ => {}
        }
    }
}

fn walk_use_tree(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    current_symbol_count: usize,
    prefix: &str,
    is_reexport: bool,
    qualified_prefix: &str,
    alias_targets: &mut Vec<(String, AliasTarget)>,
) {
    match node.kind() {
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

            let module = resolve_relative_module(&build_module_path(prefix, &path), qualified_prefix);
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport,
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module: if module.is_empty() {
                    None
                } else {
                    Some(module)
                },
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }

        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let new_prefix = build_module_path(prefix, &path);

            if let Some(list) = node.child_by_field_name("list") {
                walk_use_tree(
                    &list,
                    source,
                    refs,
                    symbols,
                    current_symbol_count,
                    &new_prefix,
                    is_reexport,
                    qualified_prefix,
                    alias_targets,
                );
            }
        }

        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "{" | "}" | "," => {}
                    _ => walk_use_tree(
                        &child,
                        source,
                        refs,
                        symbols,
                        current_symbol_count,
                        prefix,
                        is_reexport,
                        qualified_prefix,
                        alias_targets,
                    ),
                }
            }
        }

        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|n| node_text(&n, source));
            let original = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source));

            // `target_name` is the alias when present, otherwise the original name.
            let target = alias
                .clone()
                .or_else(|| original.clone())
                .unwrap_or_default();
            if target.is_empty() {
                return;
            }

            // For `use foo::bar as fb` at the top level (prefix=""), derive the
            // module from the original path: "foo::bar" → module="foo", name="bar".
            // When the alias IS the original (no `as` clause reached this arm), fall
            // back to prefix as before.
            let module = if alias.is_some() {
                // Aliased import: module = parent of the original full path.
                let orig = original.as_deref().unwrap_or("");
                let full = build_module_path(prefix, orig);
                full.rsplit_once("::")
                    .map(|(p, _)| resolve_relative_module(p, qualified_prefix))
            } else if prefix.is_empty() {
                None
            } else {
                Some(resolve_relative_module(prefix, qualified_prefix))
            };

            // Aliased imports carry the original name as a single-segment
            // chain so the SymbolIndex builder can register the alias as
            // a virtual entry pointing at the source symbol.
            let chain = if let (Some(a), Some(o)) = (alias.as_deref(), original.as_deref()) {
                if a != o && !o.is_empty() {
                    let leaf = o.rsplit("::").next().unwrap_or(o);
                    Some(crate::types::MemberChain {
                        segments: vec![crate::types::ChainSegment {
                            name: leaf.to_string(),
                            node_kind: "use_as_original".to_string(),
                            kind: crate::types::SegmentKind::Identifier,
                            declared_type: None,
                            type_args: Vec::new(),
                            optional_chaining: false,
                            byte_offset: node.start_byte() as u32,
                            declared_type_id: None,
                            is_call: false,
                            call_args: Vec::new(),
                            type_arg_ids: Vec::new(),
                        }],
                    })
                } else {
                    None
                }
            } else {
                None
            };

            // A genuine rename (`chain` is Some) that also re-exports the name
            // onto the module's surface needs a real, addressable symbol under
            // the alias: unlike an un-renamed `pub use path::Thing;`, where
            // `Thing` already IS the struct's own declared name, nothing else
            // in the package carries this alias — the package-wide bare-name
            // scan that makes the un-renamed form resolvable has nothing to
            // find for it otherwise.
            //
            // The synthetic also needs an `AliasTarget` so a member chase through
            // it (`Alias::new().touch()`) can see through to the renamed symbol's
            // own members — an `Application` target with no args, the same shape
            // a plain `type X = Y;` classifies to in TypeScript. `root` is the
            // path's trailing segment (`Thing`, not `path::Thing`): the target
            // symbol's own qualified name never carries the `use`'s module path
            // (each file's extraction starts its qualified-name prefix fresh),
            // so the bare leaf is what actually matches it — the same bare name
            // the un-renamed case's package-wide scan already relies on.
            if is_reexport && chain.is_some() {
                if let Some(alias_name) = alias.clone() {
                    if !symbols.iter().any(|s| s.name == alias_name) {
                        let alias_node = node.child_by_field_name("alias");
                        let start = alias_node
                            .map(|n| n.start_position())
                            .unwrap_or_else(|| node.start_position());
                        let end = alias_node
                            .map(|n| n.end_position())
                            .unwrap_or_else(|| node.end_position());
                        let qname = qualify(&alias_name, qualified_prefix);
                        symbols.push(ExtractedSymbol {
                            name: alias_name.clone(),
                            qualified_name: qname.clone(),
                            kind: SymbolKind::TypeAlias,
                            visibility: Some(Visibility::Public),
                            start_line: start.row as u32,
                            end_line: end.row as u32,
                            start_col: start.column as u32,
                            end_col: end.column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_from_prefix(qualified_prefix),
                            parent_index: None,
                            byte_offset: node.start_byte() as u32,
                            declared_type: None,
                            return_type: None,
                            param_types: Vec::new(),
                            generic_params: Vec::new(),
                        });
                        if let Some(orig) = original.as_deref() {
                            let root = orig.rsplit("::").next().unwrap_or(orig).to_string();
                            alias_targets.push((
                                qname,
                                AliasTarget::Application {
                                    root,
                                    args: Vec::new(),
                                },
                            ));
                        }
                    }
                }
            }

            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport,
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }

        "use_wildcard" => {
            // tree-sitter-rust shape: `(_path "::")? "*"`. The path before the
            // `*` is the node's first NAMED child (`crate` / `super` / `self` /
            // `identifier` / `scoped_identifier`); the `::` and `*` are anonymous
            // tokens. The inherited `prefix` is empty for a top-level
            // `use super::*` and carries the scoped-list prefix for a
            // `use a::{b::*}`, so combine the two. Reading the own path child is
            // what keeps `super` / `crate` / `crate::x` off NULL.
            let own_path = node
                .named_child(0)
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let module = build_module_path(prefix, &own_path);
            let module = if module.is_empty() {
                None
            } else {
                Some(module)
            };
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport,
                source_symbol_index: current_symbol_count,
                target_name: "*".to_string(),
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }

        "identifier" => {
            let name = node_text(node, source);
            if name.is_empty() {
                return;
            }
            let module = if prefix.is_empty() {
                None
            } else {
                Some(resolve_relative_module(prefix, qualified_prefix))
            };
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport,
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }

        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_use_tree(
                    &child,
                    source,
                    refs,
                    symbols,
                    current_symbol_count,
                    prefix,
                    is_reexport,
                    qualified_prefix,
                    alias_targets,
                );
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

/// Rewrite a `use` path's leading `self` / `super`(`::super`)* keyword to its
/// absolute, crate-rooted equivalent — `crate` or `crate::a::b` — using the
/// enclosing module's dot-joined `qualified_prefix`. Each `super` pops one
/// segment off `qualified_prefix`; a leading `self` consumes it unchanged.
/// A path with no such leading keyword (an external crate name, or one
/// already rooted at `crate`) passes through unmodified — this is the same
/// absolute form `self_package_sub_path` already strips `crate` from, so a
/// `use super::X` inside a nested module resolves through the same module
/// lookup a `use crate::a::X` does.
fn resolve_relative_module(module: &str, qualified_prefix: &str) -> String {
    let segs: Vec<&str> = module.split("::").collect();
    match segs.first() {
        Some(&"self") | Some(&"super") => {}
        _ => return module.to_string(),
    }
    let mut base: Vec<&str> = if qualified_prefix.is_empty() {
        Vec::new()
    } else {
        qualified_prefix.split('.').collect()
    };
    let mut i = 0;
    if segs[0] == "self" {
        i = 1;
    } else {
        while i < segs.len() && segs[i] == "super" {
            base.pop();
            i += 1;
        }
    }
    let rest = &segs[i..];
    if base.is_empty() && rest.is_empty() {
        return "crate".to_string();
    }
    let mut out = String::from("crate");
    for seg in base.iter().chain(rest.iter()) {
        out.push_str("::");
        out.push_str(seg);
    }
    out
}
