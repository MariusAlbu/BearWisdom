// =============================================================================
// php/imports.rs  —  Import-ref extraction and post-extraction ref finalization
//
// Everything that produces or reshapes an `EdgeKind::Imports` edge: `use`
// declarations (simple, aliased, grouped, `function`/`const`), `require`/
// `include`, and the file-wide pass that rewrites a usage ref's aliased local
// name back to the name its target actually declares.
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

/// Walk a `namespace_use_declaration` node, emitting one `Imports` edge per
/// imported name. Handles the simple comma-separated form (`use A\B, C\D;`,
/// each import its own `namespace_use_clause` child) and the grouped form
/// (`use A\{B, C as D};`, a `namespace_name` prefix child followed by a
/// `namespace_use_group` child whose own `namespace_use_clause` children
/// carry only their own trailing segment).
pub(super) fn extract_use_declaration(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    let mut group_prefix: Option<String> = None;
    let group_is_type_binding = node.child_by_field_name("type").is_none();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => {
                push_use_clause(&child, src, None, true, refs, current_symbol_count);
            }
            "namespace_name" => {
                group_prefix = Some(node_text(&child, src));
            }
            "namespace_use_group" => {
                let mut gcursor = child.walk();
                for member in child.children(&mut gcursor) {
                    if member.kind() == "namespace_use_clause" {
                        push_use_clause(
                            &member,
                            src,
                            group_prefix.as_deref(),
                            group_is_type_binding,
                            refs,
                            current_symbol_count,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Push an Imports edge for one `namespace_use_clause`. `group_prefix` is the
/// shared namespace prefix of a grouped `use A\{B, C};` member — `None` for
/// the simple `use A\B\C;` form, whose clause already carries the full path.
/// Reads the clause's `alias` field (`as D`) so a rename surfaces through
/// `push_fq_import`.
fn push_use_clause(
    clause: &Node,
    src: &[u8],
    group_prefix: Option<&str>,
    inherited_type_binding: bool,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let is_type_binding =
        inherited_type_binding && clause.child_by_field_name("type").is_none();
    let mut cursor = clause.walk();
    let Some(name_node) = clause
        .children(&mut cursor)
        .find(|c| c.kind() == "qualified_name" || c.kind() == "name")
    else {
        return;
    };
    let own_name = node_text(&name_node, src);
    let full = match group_prefix {
        Some(prefix) => format!("{prefix}\\{own_name}"),
        None => own_name,
    };
    let alias = clause
        .child_by_field_name("alias")
        .map(|a| node_text(&a, src));
    push_fq_import(
        full,
        alias,
        is_type_binding,
        clause.start_position().row as u32,
        clause.start_byte() as u32,
        refs,
        current_symbol_count,
    );
}

/// Push an Imports edge for a fully-qualified PHP name like `Foo\Bar\Baz`,
/// optionally renamed by an `as` clause. A rename (`alias` differs from the
/// name's last segment) carries `target_name` as the LOCAL bound name and the
/// ORIGINAL declared name as a single-segment chain — the shape
/// `build_file_context`'s `FromModuleField` rename detection expects: it
/// keys the import table entry under the original name with the bound name
/// as `alias`.
fn push_fq_import(
    full: String,
    alias: Option<String>,
    is_type_binding: bool,
    line: u32,
    byte_offset: u32,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let parts: Vec<&str> = full.split('\\').collect();
    let original = parts.last().unwrap_or(&full.as_str()).to_string();
    let module = if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("\\"))
    } else {
        None
    };

    let preserve_binding_shape =
        is_type_binding || alias.as_deref().is_some_and(|bound| bound != original);
    let chain = if preserve_binding_shape {
        Some(MemberChain {
            segments: vec![ChainSegment {
                name: original.clone(),
                node_kind: if is_type_binding {
                    "namespace_use_type"
                } else {
                    "namespace_use_value"
                }
                .to_string(),
                kind: if is_type_binding {
                    SegmentKind::TypeAccess
                } else {
                    SegmentKind::Identifier
                },
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            }],
        })
    } else {
        None
    };
    let target_name = alias.unwrap_or(original);

    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: current_symbol_count,
        target_name,
        kind: EdgeKind::Imports,
        line,
        module,
        chain,
        byte_offset,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        col: 0,
    });
}

/// Extract an Imports edge from an `include`/`require`/`include_once`/`require_once` expression.
pub(super) fn extract_include_require(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    // The path expression is the only named child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "encapsed_string" {
            let raw = node_text(&child, src);
            // Strip surrounding quotes.
            let path = raw
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('\'')
                .to_string();
            if path.is_empty() {
                continue;
            }
            let parts: Vec<&str> = path.split('/').collect();
            let target = parts
                .last()
                .unwrap_or(&path.as_str())
                .trim_end_matches(".php")
                .to_string();
            let module = if parts.len() > 1 {
                Some(parts[..parts.len() - 1].join("/"))
            } else {
                None
            };
            refs.push(ExtractedRef {
                is_include: false,
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index,
                target_name: target,
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
    }
}

// ---------------------------------------------------------------------------
// Post-extraction ref finalization
// ---------------------------------------------------------------------------

/// Rewrite a bare-name usage ref's `target_name` from a locally aliased
/// import's bound name back to the name the target actually declares
/// (`use Foo\Bar as Baz;` then `new Baz()` rewrites to `target_name: "Bar"`).
/// Mirrors the `target_name` invariant documented on `ExtractedRef`: the
/// canonical exported name, not the importing file's local alias.
///
/// Scoped to `Instantiates`/`Implements`/`Inherits`/`TypeRef` — grammatical
/// positions only a CLASS-shaped `use` alias can occupy. PHP keeps separate
/// namespaces for classes, functions, and constants, so a `use function`/`use
/// const` alias must never rewrite a `Calls` ref: a same-named function or
/// constant unrelated to the aliased class could collide.
fn apply_use_aliases(refs: &mut [ExtractedRef]) {
    let aliases: std::collections::HashMap<String, String> = refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .filter_map(|r| {
            let seg = r.chain.as_ref()?.segments.first()?;
            if seg.kind != SegmentKind::TypeAccess {
                return None;
            }
            Some((r.target_name.clone(), seg.name.clone()))
        })
        .collect();
    if aliases.is_empty() {
        return;
    }
    for r in refs.iter_mut() {
        if !matches!(
            r.kind,
            EdgeKind::Instantiates | EdgeKind::Implements | EdgeKind::Inherits | EdgeKind::TypeRef
        ) {
            continue;
        }
        if let Some(original) = aliases.get(&r.target_name) {
            r.target_name = original.clone();
        }
    }
}

/// Rewrite aliased-usage refs, then drop exact duplicates. `scan_all_type_refs`
/// overlaps with the per-arm walk in `extract_from_node`, and the alias
/// rewrite can additionally collapse two previously-distinct target names
/// onto the same one — both sources of duplication are resolved here, in
/// that order, so the dedup key sees the final `target_name`. Key includes
/// `module` so refs with different qualifier paths survive.
pub(super) fn finalize_refs(refs: &mut Vec<ExtractedRef>) {
    append_qualified_import_type_refs(refs);
    apply_use_aliases(refs);

    let mut seen: std::collections::HashSet<(usize, String, EdgeKind, u32, u32, Option<String>)> =
        std::collections::HashSet::with_capacity(refs.len());
    refs.retain(|r| {
        seen.insert((
            r.source_symbol_index,
            r.target_name.clone(),
            r.kind,
            r.line,
            r.byte_offset,
            r.module.clone(),
        ))
    });
}

/// Add a direct type demand for a qualified static receiver whose first
/// component is a class/namespace import. Composer indexes real declarations
/// by `(namespace, type)`; this preserves that exact pair without inventing a
/// representative file for the imported namespace directory.
fn append_qualified_import_type_refs(refs: &mut Vec<ExtractedRef>) {
    let imports: std::collections::HashMap<String, String> = refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .filter_map(|r| {
            let segment = r.chain.as_ref()?.segments.first()?;
            if segment.kind != SegmentKind::TypeAccess {
                return None;
            }
            let module = r.module.as_deref()?;
            Some((
                r.target_name.clone(),
                format!("{module}\\{}", segment.name),
            ))
        })
        .collect();
    if imports.is_empty() {
        return;
    }

    let mut demands = Vec::new();
    for reference in refs.iter() {
        let Some(root) = reference.chain.as_ref().and_then(|chain| chain.segments.first()) else {
            continue;
        };
        if root.kind != SegmentKind::TypeAccess {
            continue;
        }
        let Some((binding, tail)) = root.name.split_once('\\') else {
            continue;
        };
        let Some(imported_namespace) = imports.get(binding) else {
            continue;
        };
        let Some((owner_tail, type_name)) = tail.rsplit_once('\\') else {
            demands.push((
                reference.source_symbol_index,
                reference.line,
                reference.col,
                reference.byte_offset,
                imported_namespace.clone(),
                tail.to_string(),
            ));
            continue;
        };
        demands.push((
            reference.source_symbol_index,
            reference.line,
            reference.col,
            reference.byte_offset,
            format!("{imported_namespace}\\{owner_tail}"),
            type_name.to_string(),
        ));
    }

    for (source_symbol_index, line, col, byte_offset, module, target_name) in demands {
        if target_name.is_empty() {
            continue;
        }
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name,
            kind: EdgeKind::TypeRef,
            line,
            col,
            module: Some(module),
            chain: None,
            byte_offset,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

#[cfg(test)]
#[path = "imports_tests.rs"]
mod tests;
