// =============================================================================
// ecosystem/ecmascript_imports.rs — shared TS/JS import-extraction layer
//
// Both `languages/typescript/imports.rs` and `languages/javascript/extract.rs`
// previously held byte-for-byte copies of `push_import` and `build_import_map`.
// `tree-sitter-javascript` is a strict subset of `tree-sitter-typescript`
// (same node kinds, same field names) so the two extractors don't actually
// disagree about how to walk an `import_statement` — the divergence was
// just code drift. This module is the single source of truth.
//
// **Scope.** Extractor-side import handling only:
//   - Walk an `import_statement` AST node and emit Imports / TypeRef refs.
//   - Walk all top-level imports of a file and produce the local-name →
//     `ImportEntry` map consumed by `ecosystem::imports::resolve_import_refs`.
//
// The downstream resolver (`ecosystem::imports`) is a separate concern and
// stays where it is. This module sits one layer earlier — it builds the
// inputs the resolver consumes.
//
// **Behavior parity.** TS and JS were not 100% identical going in:
//   - JS emits a coverage `Imports` ref pinned to the import_statement's
//     start line so the per-line coverage budget is satisfied even when
//     all named specifiers are on subsequent lines. TS didn't.
//   - JS emits a side-effect fallback (`import './styles.css'` with no
//     clause body) as an `Imports` ref. TS didn't.
//   - TS handles `import path = require("mod")` (`import_require_clause`).
//     JS doesn't have this construct.
//
// `PushImportOpts` captures these so the refactor is pure dedup with no
// behavior change. We can flip flags later as a deliberate, separate
// quality move (e.g., enabling JS-style coverage on TS).
// =============================================================================

use std::collections::HashMap;

use tree_sitter::Node;

use crate::ecosystem::imports::{ImportEntry, ImportKind};
use crate::types::{EdgeKind, ExtractedRef};

/// Toggles for the language-specific differences in import-ref emission.
/// See module-level comment for the per-flag rationale.
#[derive(Debug, Clone, Copy)]
pub struct PushImportOpts {
    /// Emit one `Imports` ref pinned to the import_statement's start line.
    /// JS uses this; TS hasn't historically. Coverage-engine concern.
    pub emit_line_imports: bool,
    /// Emit a side-effect-import `Imports` ref when the statement has no
    /// `import_clause` (e.g. `import './styles.css'`). JS uses this.
    pub emit_side_effect_fallback: bool,
    /// Walk `import_require_clause` children for TS's
    /// `import path = require("mod")` shape. TS-only.
    pub handle_import_require_clause: bool,
}

impl PushImportOpts {
    pub const TYPESCRIPT: Self = Self {
        emit_line_imports: false,
        emit_side_effect_fallback: false,
        handle_import_require_clause: true,
    };

    pub const JAVASCRIPT: Self = Self {
        emit_line_imports: true,
        emit_side_effect_fallback: true,
        handle_import_require_clause: false,
    };
}

/// Walk an `import_statement` AST node, emitting Imports / TypeRef refs.
/// Both `languages/typescript/imports.rs::push_import` and
/// `languages/javascript/extract.rs::push_import` delegate here.
pub fn push_import_refs(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
    opts: PushImportOpts,
) {
    let module_path = node
        .child_by_field_name("source")
        .map(|s| trimmed_string(s, src));

    let line = node.start_position().row as u32;

    // JS-style: always emit an Imports ref at the statement's start line
    // for coverage. TS doesn't currently do this — flag-gated.
    if opts.emit_line_imports {
        if let Some(mod_path) = &module_path {
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: mod_path.clone(),
                kind: EdgeKind::Imports,
                line,
                module: module_path.clone(),
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }

    let initial_ref_count = refs.len();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                emit_clause_refs(&child, src, current_symbol_count, &module_path, refs);
            }
            "import_require_clause" if opts.handle_import_require_clause => {
                emit_require_clause_ref(&child, src, current_symbol_count, refs);
            }
            _ => {}
        }
    }

    // JS-style side-effect fallback: `import './styles.css'`.
    if opts.emit_side_effect_fallback && refs.len() == initial_ref_count {
        if let Some(mod_path) = &module_path {
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: mod_path.clone(),
                kind: EdgeKind::Imports,
                line,
                module: module_path.clone(),
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}

fn emit_clause_refs(
    clause: &Node,
    src: &[u8],
    sym_idx: usize,
    module_path: &Option<String>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = clause.walk();
    for item in clause.children(&mut cursor) {
        match item.kind() {
            // `import Foo from 'pkg'` — default import.
            "identifier" => {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: text_of(item, src),
                    kind: EdgeKind::TypeRef,
                    line: item.start_position().row as u32,
                    module: module_path.clone(),
                    chain: None,
                    byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            // `import { X, Y as Z } from 'pkg'` — emit per import_specifier
            // using the imported (exported) name as target so the resolver
            // can match against the source module's export list.
            "named_imports" => {
                let mut ni = item.walk();
                for spec in item.children(&mut ni) {
                    if spec.kind() != "import_specifier" {
                        continue;
                    }
                    let imported_name = spec
                        .child_by_field_name("name")
                        .map(|n| text_of(n, src))
                        .unwrap_or_else(|| text_of(spec, src));
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: imported_name,
                        kind: EdgeKind::TypeRef,
                        line: spec.start_position().row as u32,
                        module: module_path.clone(),
                        chain: None,
                        byte_offset: 0,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            // `import * as ns from 'pkg'`.
            "namespace_import" => {
                let mut nc = item.walk();
                for ns_child in item.children(&mut nc) {
                    if ns_child.kind() == "identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: text_of(ns_child, src),
                            kind: EdgeKind::TypeRef,
                            line: ns_child.start_position().row as u32,
                            module: module_path.clone(),
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

/// `import path = require("mod")` — TS CommonJS-interop shape.
/// Children are: identifier (local), `=`, `require`, `(`, string, `)`.
fn emit_require_clause_ref(
    clause: &Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut local_name = String::new();
    let mut require_module: Option<String> = None;
    let mut rc = clause.walk();
    for rc_child in clause.children(&mut rc) {
        match rc_child.kind() {
            "identifier" if local_name.is_empty() => {
                local_name = text_of(rc_child, src);
            }
            "string" => {
                require_module = Some(trimmed_string(rc_child, src));
            }
            _ => {}
        }
    }
    if !local_name.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index: sym_idx,
            target_name: local_name,
            kind: EdgeKind::Imports,
            line: clause.start_position().row as u32,
            module: require_module,
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

// ---------------------------------------------------------------------------
// build_import_map — produces the local-name → ImportEntry table that
// `ecosystem::imports::resolve_import_refs` consumes.
//
// TS and JS extractors had byte-identical copies of this. Single source.
// ---------------------------------------------------------------------------

pub fn build_import_map(root: Node, src: &[u8]) -> HashMap<String, ImportEntry> {
    let mut map: HashMap<String, ImportEntry> = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "import_statement" {
            continue;
        }
        let Some(module_node) = child.child_by_field_name("source") else {
            continue;
        };
        let module_path = trimmed_string(module_node, src);
        if module_path.is_empty() {
            continue;
        }

        let mut ic = child.walk();
        for clause in child.children(&mut ic) {
            if clause.kind() != "import_clause" {
                continue;
            }
            insert_clause_entries(&clause, src, &module_path, &mut map);
        }
    }
    map
}

fn insert_clause_entries(
    clause: &Node,
    src: &[u8],
    module_path: &str,
    map: &mut HashMap<String, ImportEntry>,
) {
    let mut cc = clause.walk();
    for item in clause.children(&mut cc) {
        match item.kind() {
            // `import Foo from 'pkg'` — default import.
            "identifier" => {
                let local = text_of(item, src);
                if !local.is_empty() {
                    map.insert(
                        local.clone(),
                        ImportEntry {
                            local_name: local,
                            module: module_path.to_string(),
                            kind: ImportKind::Default,
                        },
                    );
                }
            }
            // `import { Foo, Bar as B } from 'pkg'`
            "named_imports" => {
                let mut ni = item.walk();
                for spec in item.children(&mut ni) {
                    if spec.kind() != "import_specifier" {
                        continue;
                    }
                    let exported = spec
                        .child_by_field_name("name")
                        .map(|n| text_of(n, src))
                        .unwrap_or_default();
                    let local = spec
                        .child_by_field_name("alias")
                        .map(|n| text_of(n, src))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| exported.clone());
                    if local.is_empty() || exported.is_empty() {
                        continue;
                    }
                    map.insert(
                        local.clone(),
                        ImportEntry {
                            local_name: local,
                            module: module_path.to_string(),
                            kind: ImportKind::Named { exported_name: exported },
                        },
                    );
                }
            }
            // `import * as ns from 'pkg'`
            "namespace_import" => {
                let mut nc = item.walk();
                for ns_child in item.children(&mut nc) {
                    if ns_child.kind() == "identifier" {
                        let local = text_of(ns_child, src);
                        if !local.is_empty() {
                            map.insert(
                                local.clone(),
                                ImportEntry {
                                    local_name: local,
                                    module: module_path.to_string(),
                                    kind: ImportKind::Namespace,
                                },
                            );
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tiny helpers — both languages had `node_text` in their own helpers.rs.
// We use a local copy here to avoid taking a dependency on either language
// crate's helpers from the ecosystem layer.
// ---------------------------------------------------------------------------

fn text_of(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

fn trimmed_string(node: Node, src: &[u8]) -> String {
    text_of(node, src)
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

