// =============================================================================
// python/imports.rs  —  Import-ref extraction and the local→module name map
//
// Everything that produces an `EdgeKind::Imports` edge (`import foo`,
// `import foo.bar as fb`, `from a.b import c as d`, `from . import x`,
// wildcard `from m import *`) plus `build_import_map`, the local-name →
// dotted-module-path table `extract_calls_from_body` uses to attach `module`
// onto qualified call refs (`Person.objects.filter()` → `posthog.models`).
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use rustc_hash::FxHashSet;
use std::collections::HashMap;
use tree_sitter::Node;

/// Build a map from local name → fully-qualified module path by scanning the
/// immediate children of `root` for `import_statement` and
/// `import_from_statement` nodes.
///
/// Mapping rules:
/// - `import json`           → `"json"  → "json"`
/// - `import foo.bar`        → `"foo"   → "foo.bar"` (first segment is local)
/// - `import foo.bar as fb`  → `"fb"    → "foo.bar"`
/// - `from foo.bar import Baz`     → `"Baz" → "foo.bar"`
/// - `from foo import bar as b`    → `"b"   → "foo"`
/// - `from . import x`             → `"x"   → "."`  (dots preserved)
/// - `from ..pkg import w`         → `"w"   → "..pkg"`
///
/// Only the top-level module scope is scanned (not function bodies), which is
/// where almost all Python imports live.
pub(super) fn build_import_map(root: tree_sitter::Node, source: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    match item.kind() {
                        "dotted_name" => {
                            let full = node_text(&item, source);
                            // `import foo.bar` — local name is the first segment
                            let local = full.split('.').next().unwrap_or(&full).to_string();
                            map.insert(local, full);
                        }
                        "aliased_import" => {
                            // `import foo.bar as fb`
                            if let (Some(name_node), Some(alias_node)) = (
                                item.child_by_field_name("name"),
                                item.child_by_field_name("alias"),
                            ) {
                                let full = node_text(&name_node, source);
                                let alias = node_text(&alias_node, source);
                                if !alias.is_empty() {
                                    map.insert(alias, full);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "import_from_statement" => {
                // `module_name` is a required field: either a `dotted_name`
                // (absolute — no leading dot) or a `relative_import` node
                // whose own span already covers the leading dot(s) plus any
                // trailing name (`.`, `..`, `.rel`, `..pkg`). The raw text is
                // kept as-is so a relative specifier stays distinguishable
                // from an absolute one downstream.
                let module = match child.child_by_field_name("module_name") {
                    Some(m) => node_text(&m, source),
                    None => continue,
                };
                let module_id = child.child_by_field_name("module_name").map(|n| n.id());

                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    if module_id.map_or(false, |id| item.id() == id) {
                        continue;
                    }
                    match item.kind() {
                        "dotted_name" | "identifier" => {
                            let name = node_text(&item, source);
                            if !name.is_empty() {
                                map.insert(name, module.clone());
                            }
                        }
                        "aliased_import" => {
                            // `from foo import bar as b`
                            if let Some(alias_node) = item.child_by_field_name("alias") {
                                let alias = node_text(&alias_node, source);
                                if !alias.is_empty() {
                                    map.insert(alias, module.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Import reference extraction
// ---------------------------------------------------------------------------

/// A single-segment chain carrying `declared_name` — the shape
/// `build_file_context`'s `FromModuleField` rename detection expects: it
/// keys the import table entry under the chain's name (the module's own
/// declared name) with `target_name` as the locally bound alias.
fn rename_chain(declared_name: &str, byte_offset: u32) -> MemberChain {
    MemberChain {
        segments: vec![ChainSegment {
            name: declared_name.to_string(),
            node_kind: "import_as_original".to_string(),
            kind: SegmentKind::Identifier,
            declared_type: None,
            type_args: Vec::new(),
            optional_chaining: false,
            byte_offset,
            declared_type_id: None,
            is_call: false,
            call_args: Vec::new(),
            type_arg_ids: Vec::new(),
        }],
    }
}

/// Choose the `(target_name, chain)` pair for an `as`-aliased import.
///
/// A re-export (`__all__` lists the bound name) keeps `target_name` as the
/// module's own DECLARED name with no chain — the shape the file-level
/// re-export map reads directly off `target_name` + `module` as a
/// cross-module hop (`export { X } from 'm'`); rewriting it to the alias
/// would desync that pass, which never consults `chain`.
///
/// A private alias (not re-exported) carries `target_name` as the LOCAL
/// bound name instead, with `declared_name` preserved via `rename_chain` —
/// this is what lets a bare usage of the alias resolve through the file's
/// import table. Also used when there is no actual rename (`local ==
/// declared_name`), where the chain is redundant and omitted.
fn aliased_target(
    declared_name: String,
    local: &str,
    is_reexport: bool,
    byte_offset: u32,
) -> (String, Option<MemberChain>) {
    if is_reexport || local == declared_name {
        (declared_name, None)
    } else {
        let chain = rename_chain(&declared_name, byte_offset);
        (local.to_string(), Some(chain))
    }
}

/// Walk an `import_statement` node (`import foo`, `import foo.bar`,
/// `import foo.bar as fb`), emitting one `Imports` edge per imported name.
pub(super) fn extract_import_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    dunder_all: &FxHashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let full = node_text(&child, source);
                let parts: Vec<&str> = full.split('.').collect();
                let target = parts.last().unwrap_or(&full.as_str()).to_string();
                // `import foo.bar` carries the parent package as `module`; a
                // single-segment `import foo` has no parent, so `module` names
                // the imported module itself — the file-path matcher resolves
                // it the same way a dotted module resolves its last segment.
                let module = if parts.len() > 1 {
                    Some(parts[..parts.len() - 1].join("."))
                } else {
                    Some(target.clone())
                };
                // `import foo.bar` binds the TOP segment `foo` as the local name.
                let local = parts.first().copied().unwrap_or(full.as_str());
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: dunder_all.contains(local),
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let full = node_text(&name_node, source);
                    let parts: Vec<&str> = full.split('.').collect();
                    let declared = parts.last().unwrap_or(&full.as_str()).to_string();
                    // Same single-segment rule as the unaliased form above:
                    // `import foo as f` names `foo` itself as `module`.
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("."))
                    } else {
                        Some(declared.clone())
                    };
                    let local = child
                        .child_by_field_name("alias")
                        .map(|a| node_text(&a, source))
                        .unwrap_or_else(|| {
                            parts.first().map(|s| s.to_string()).unwrap_or_default()
                        });
                    let is_reexport = dunder_all.contains(&local);
                    let byte_offset = child.start_byte() as u32;
                    let (target_name, chain) =
                        aliased_target(declared, &local, is_reexport, byte_offset);
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport,
                        source_symbol_index: current_symbol_count,
                        target_name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        col: 0,
                        module,
                        chain,
                        byte_offset,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
}

/// Walk an `import_from_statement` node (`from a.b import c`, `from a import
/// b as d`, `from . import x`, `from m import *`), emitting one `Imports`
/// edge per imported name.
pub(super) fn extract_import_from_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    dunder_all: &FxHashSet<String>,
) {
    // `module_name` is required and is either a `dotted_name` (absolute) or
    // a `relative_import` node whose span already covers the leading dot(s)
    // — kept verbatim so `.rel`/`..pkg`/`.` stay distinguishable from an
    // absolute module downstream (the relative-marker check the resolver
    // runs is a literal `starts_with('.')`).
    let module = node
        .child_by_field_name("module_name")
        .map(|m| node_text(&m, source));

    let module_name_node = node.child_by_field_name("module_name");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "from" | "import" | "," | "import_prefix" => continue,
            _ => {}
        }
        if let Some(ref mn) = module_name_node {
            if child.id() == mn.id() {
                continue;
            }
        }

        match child.kind() {
            "dotted_name" | "identifier" => {
                let name = node_text(&child, source);
                // `from .mod import A` binds `A` (the imported name) locally.
                let is_reexport = dunder_all.contains(&name);
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport,
                    source_symbol_index: current_symbol_count,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: module.clone(),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let declared = node_text(&name_node, source);
                    let local = child
                        .child_by_field_name("alias")
                        .map(|a| node_text(&a, source))
                        .unwrap_or_else(|| declared.clone());
                    let is_reexport = dunder_all.contains(&local);
                    let byte_offset = child.start_byte() as u32;
                    let (target_name, chain) =
                        aliased_target(declared, &local, is_reexport, byte_offset);
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport,
                        source_symbol_index: current_symbol_count,
                        target_name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: module.clone(),
                        chain,
                        byte_offset,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            "wildcard_import" => {
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: current_symbol_count,
                    target_name: "*".to_string(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: module.clone(),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "imports_tests.rs"]
mod tests;
