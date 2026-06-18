// =============================================================================
// languages/typescript/reexports.rs — re-export ref emission + triple-slash directives
// =============================================================================

use super::helpers;
use crate::ecosystem::imports::{ImportEntry, ImportKind};
use crate::types::{EdgeKind, ExtractedRef};
use std::collections::HashMap;
use tree_sitter::Node;

pub(super) fn extract_bare_reexports_via_imports(
    root: Node,
    src: &[u8],
    import_map: &HashMap<String, ImportEntry>,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "export_statement" {
            continue;
        }
        // The with-source path is owned by `extract_reexports`. Skip it
        // here so we don't double-emit Imports refs for the same clause.
        if child.child_by_field_name("source").is_some() {
            continue;
        }

        // `export type { … }` — the whole clause is type-only. The
        // grammar surfaces `type` as a top-level keyword child of the
        // export_statement, before the export_clause.
        let stmt_type_only = (0..child.child_count()).any(|i| {
            child
                .child(i)
                .map(|c| c.kind() == "type" && i < 2)
                .unwrap_or(false)
        });

        let mut ec = child.walk();
        for clause in child.children(&mut ec) {
            if clause.kind() != "export_clause" {
                continue;
            }
            let mut sc = clause.walk();
            for spec in clause.children(&mut sc) {
                if spec.kind() != "export_specifier" {
                    continue;
                }
                let name = spec
                    .child_by_field_name("name")
                    .map(|n| helpers::node_text(n, src))
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }

                let Some(import) = import_map.get(&name) else {
                    // `name` is not imported — it's a locally-declared symbol
                    // re-exported under a new name (`export { local as exposed }`,
                    // no `from`). Record the rename as a module-less re-export ref
                    // (`target_name` = local source, `namespace_segments[0]` =
                    // exposed name) so an import-type that indexes the exposed name
                    // can follow it to the local declaration's type. A bare
                    // `export { local }` adds no name and needs no ref.
                    let alias = spec
                        .child_by_field_name("alias")
                        .map(|n| helpers::node_text(n, src))
                        .unwrap_or_default();
                    let declared_locally = symbols.iter().any(|s| s.name == name);
                    if !alias.is_empty() && declared_locally {
                        let source_idx = symbols.len().saturating_sub(1);
                        refs.push(ExtractedRef {
                            is_import_binding: false,
                            is_reexport: true,
                            source_symbol_index: source_idx,
                            target_name: name.clone(),
                            kind: EdgeKind::Imports,
                            line: spec.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: spec.start_byte() as u32,
                            namespace_segments: vec![alias],
                            call_args: Vec::new(),
                        });
                    }
                    continue;
                };

                // The canonical name in the source module — for renamed
                // imports (`import { X as Y }`), the source's export
                // list has `X`, not `Y`. The Imports ref must encode the
                // source-side name so cross-package chain following
                // matches the actual export.
                let exported_in_source = match &import.kind {
                    ImportKind::Named { exported_name } => exported_name.clone(),
                    ImportKind::Default => name.clone(),
                    // Namespace re-exports of a `import * as ns`-style binding
                    // are too ambiguous to resolve generically — skip.
                    ImportKind::Namespace | ImportKind::SideEffect => continue,
                };

                let alias = spec
                    .child_by_field_name("alias")
                    .map(|n| helpers::node_text(n, src))
                    .unwrap_or_default();
                let exposed = if !alias.is_empty() {
                    alias.clone()
                } else {
                    name.clone()
                };

                // Per-specifier `type` modifier: `export { type X }`.
                let spec_type_only = (0..spec.child_count())
                    .any(|i| spec.child(i).map(|c| c.kind() == "type").unwrap_or(false));
                let type_only = stmt_type_only || spec_type_only;

                let source_idx = symbols.len().saturating_sub(1);
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: true,
                    source_symbol_index: source_idx,
                    target_name: exported_in_source,
                    kind: EdgeKind::Imports,
                    line: spec.start_position().row as u32,
                    col: 0,
                    module: Some(import.module.clone()),
                    chain: None,
                    byte_offset: spec.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });

                let already_emitted = symbols.iter().any(|s| s.qualified_name == exposed);
                if !already_emitted {
                    let kind = if type_only {
                        SymbolKind::TypeAlias
                    } else {
                        // Variable matches both TypeRef and Calls in
                        // `predicates::kind_compatible`, so the consumer
                        // ref resolves regardless of how downstream
                        // names this re-export.
                        SymbolKind::Variable
                    };
                    symbols.push(ExtractedSymbol {
                        name: exposed.clone(),
                        qualified_name: exposed,
                        kind,
                        visibility: Some(Visibility::Public),
                        start_line: spec.start_position().row as u32 + 1,
                        end_line: spec.end_position().row as u32 + 1,
                        start_col: spec.start_position().column as u32,
                        end_col: spec.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: None,
                        parent_index: None,
                        byte_offset: 0,
                        declared_type: None,
                        return_type: None,
                        param_types: Vec::new(),
                        generic_params: Vec::new(),
                    });
                }
            }
        }
    }
}

/// `ecosystem::imports::resolve_import_refs` pass, which canonicalizes
/// every ref against this map before the file is handed to the resolver.
///
/// Scan the head of a TypeScript source for triple-slash directives:
///
///   `/// <reference path="X" />`    — sibling-file include (relative)
///   `/// <reference types="pkg" />` — npm typings include
///
/// Each match emits an `EdgeKind::Imports` ref with `module = Some(spec)`,
/// which feeds into the standard import resolution path. Path-form refs
/// resolve against the source file's directory; types-form refs resolve
/// against `node_modules/@types/<spec>` (the npm walker's existing
/// @types fallback handles them).
///
/// The scan stops at the first non-comment / non-blank line per the TS
/// language spec — directives must precede all source.
pub(super) fn push_triple_slash_imports(source: &str, refs: &mut Vec<ExtractedRef>) {
    let mut byte_offset: usize = 0;
    for (line_no, line) in source.lines().enumerate() {
        let line_byte_offset = byte_offset;
        // Advance past this line's bytes plus the newline separator.
        byte_offset += line.len() + 1;
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // Per the TS spec, triple-slash directives must precede all
        // statements; bail at the first non-comment line so we don't
        // pick up `/// XXX` written inside a JSDoc block far below.
        if !trimmed.starts_with("///") {
            // Allow JSDoc-style block comments and regular `//` comments
            // before the first directive — many .d.ts files have a
            // license header.
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                continue;
            }
            break;
        }
        let body = trimmed.trim_start_matches('/').trim();
        // Match `<reference path="X" />` and `<reference types="X" />`.
        // The exact form is: `<reference KIND="VALUE" />` with optional
        // additional attributes and varying whitespace.
        let Some(rest) = body.strip_prefix("<reference") else {
            continue;
        };
        let rest = rest.trim_start();
        for kind in &["path", "types"] {
            let prefix = format!("{kind}=");
            let Some(after) = rest.strip_prefix(&prefix) else {
                // Try after a leading `lib=` or other attribute.
                let probe = rest.find(&prefix);
                if let Some(idx) = probe {
                    if !is_attribute_boundary(rest.as_bytes(), idx) {
                        continue;
                    }
                    if let Some(value) = read_quoted(&rest[idx + prefix.len()..]) {
                        emit_triple_slash_ref(refs, kind, &value, line_no, line_byte_offset);
                    }
                }
                continue;
            };
            if let Some(value) = read_quoted(after) {
                emit_triple_slash_ref(refs, kind, &value, line_no, line_byte_offset);
            }
        }
    }
}

fn read_quoted(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let q = bytes[0];
    if q != b'"' && q != b'\'' {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i] != q {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    if i >= bytes.len() {
        return None;
    }
    Some(s[1..i].to_string())
}

/// Char before `idx` must be a word boundary (whitespace or `<`) so we
/// don't match `pathy="X"` thinking it's `path="X"`.
fn is_attribute_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    prev == b' ' || prev == b'\t' || prev == b'<' || prev == b'\n'
}

fn emit_triple_slash_ref(
    refs: &mut Vec<ExtractedRef>,
    kind: &str,
    value: &str,
    line: usize,
    byte_offset: usize,
) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    // For `types="pkg"`, rewrite to `@types/pkg/<entry>` so the npm
    // walker matches against the actual package layout. For `path="X"`,
    // pass the relative spec through and let the file-stem fallback in
    // resolve_common locate it.
    let module = match kind {
        "types" => format!("@types/{value}"),
        _ => value.to_string(),
    };
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: module.clone(),
        kind: EdgeKind::Imports,
        line: line as u32,
        col: 0,
        module: Some(module),
        chain: None,
        byte_offset: byte_offset as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// Covers:
///   - `import Foo from 'pkg'`             → Default
///   - `import { X } from 'pkg'`           → Named { exported_name=X }
///   - `import { X as Y } from 'pkg'`      → Named { exported_name=X } keyed under Y
///   - `import * as ns from 'pkg'`         → Namespace
///   - `import 'pkg'`                      → SideEffect (no local name; not stored)
// build_import_map moved to crate::ecosystem::ecmascript_imports — both TS
// and JS extractors now share that single implementation.

/// Extract re-export refs from an `export_statement` node.
///
/// Handles:
///   `export { X } from './y'`              → Imports ref, target_name="X", module="./y"
///   `export { X as Z } from './y'`         → Imports ref + synthetic Z symbol so
///                                            consumers' `import { Z }` resolves.
///   `export * from './y'`                  → Imports ref, target_name="*", module="./y"
///   `export * as ns from './y'`            → Imports ref, target_name="*", module="./y"
///
/// Re-exports are attributed to a file-level "sentinel" symbol at index
/// `file_symbol_count` — the index one past the last real symbol, which is
/// how the JS extractor handles them.  If the file has no symbols yet,
/// index 0 is fine because the resolution engine only uses the module field.
///
/// **Why we emit synthetic symbols for renamed re-exports.** A consumer
/// that writes `import { AnyTRPCRouter } from '@trpc/server'` looks for a
/// symbol named `AnyTRPCRouter` in the package's entry chain. When the
/// definition is `export { type AnyRouter as AnyTRPCRouter } from './core'`,
/// no such symbol exists anywhere in the project — only the `AnyRouter`
/// original. Emitting an alias-named symbol here gives the resolver
/// something to land on so the import resolves; the existing Imports ref
/// continues to encode the redirection back to the source for downstream
/// chain walks that need the underlying definition.
pub(super) fn extract_reexports(
    node: &tree_sitter::Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

    // The source module is the `source` field of the export_statement.
    let module_path = node.child_by_field_name("source").map(|s| {
        helpers::node_text(s, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    });

    // Only re-export forms have a `source` field.
    let Some(ref mod_path) = module_path else {
        return;
    };
    if mod_path.is_empty() {
        return;
    }

    // Use sentinel index: one past the last symbol (or 0 if no symbols yet).
    // The resolver only needs `target_name` and `module`; the source index is
    // irrelevant for re-export chain following.
    let source_idx = symbols.len().saturating_sub(1);

    let line = node.start_position().row as u32;
    let mut has_wildcard = false;
    // `export type { ... } from '...'` — the whole clause is type-only.
    // Tree-sitter typescript surfaces this as a `type` keyword child of the
    // export_statement node itself, before the `export_clause`.
    let stmt_type_only = (0..node.child_count()).any(|i| {
        node.child(i)
            .map(|c| c.kind() == "type" && i < 2)
            .unwrap_or(false)
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `export { X }` or `export { X as Z }` from './y'
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        // `name` field = the original exported name (before `as`).
                        // `alias` field = the local rename (after `as`).
                        let original_name = spec
                            .child_by_field_name("name")
                            .map(|n| helpers::node_text(n, src))
                            .unwrap_or_default();
                        let alias_name = spec
                            .child_by_field_name("alias")
                            .map(|n| helpers::node_text(n, src))
                            .unwrap_or_default();
                        // Per-specifier `type` modifier: `export { type X as Y }`.
                        let spec_type_only = (0..spec.child_count())
                            .any(|i| spec.child(i).map(|c| c.kind() == "type").unwrap_or(false));
                        let type_only = stmt_type_only || spec_type_only;
                        if !original_name.is_empty() {
                            // The Imports ref encodes the redirection — store
                            // the original so the resolver can find it in the
                            // source module. (Unchanged from prior behavior.)
                            refs.push(ExtractedRef {
                                is_import_binding: false,
                                is_reexport: true,
                                source_symbol_index: source_idx,
                                target_name: original_name.clone(),
                                kind: EdgeKind::Imports,
                                line: spec.start_position().row as u32,
                                col: 0,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: spec.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                            });
                        }

                        // Emit a synthetic symbol named after what the export
                        // exposes (alias when present, original otherwise).
                        // Without this, consumers' `import { X } from '...'`
                        // and bare type refs to `X` find nothing in this file
                        // — the re-export is invisible at the symbol layer.
                        // The behaviour mirrors the JS extractor and gives
                        // the resolver a landing point for both renamed and
                        // bare re-exports without disturbing the existing
                        // `Imports` ref that drives cross-package chain
                        // following.
                        let exposed = if !alias_name.is_empty() {
                            alias_name.clone()
                        } else {
                            original_name.clone()
                        };
                        let already_emitted = !exposed.is_empty()
                            && symbols.iter().any(|s| s.qualified_name == exposed);
                        if !exposed.is_empty() && !already_emitted {
                            let kind = if type_only {
                                SymbolKind::TypeAlias
                            } else {
                                // Variable matches both TypeRef and Calls in
                                // `predicates::kind_compatible`, so the
                                // consumer ref resolves regardless of how
                                // the symbol is referenced downstream.
                                SymbolKind::Variable
                            };
                            symbols.push(ExtractedSymbol {
                                name: exposed.clone(),
                                qualified_name: exposed,
                                kind,
                                visibility: Some(Visibility::Public),
                                start_line: spec.start_position().row as u32 + 1,
                                end_line: spec.end_position().row as u32 + 1,
                                start_col: spec.start_position().column as u32,
                                end_col: spec.end_position().column as u32,
                                signature: None,
                                doc_comment: None,
                                scope_path: None,
                                parent_index: None,
                                byte_offset: 0,
                                declared_type: None,
                                return_type: None,
                                param_types: Vec::new(),
                                generic_params: Vec::new(),
                            });
                        }
                    }
                }
            }
            // `export * as ns from './y'` — the TS grammar wraps this in namespace_export.
            "namespace_export" => {
                has_wildcard = true;
            }
            // `export * from './y'` — in the TS grammar the `*` is a direct child of
            // export_statement (no namespace_export wrapper), unlike the JS grammar.
            "*" => {
                has_wildcard = true;
            }
            _ => {}
        }
    }

    if has_wildcard {
        refs.push(ExtractedRef {
            is_import_binding: false,
            is_reexport: true,
            source_symbol_index: source_idx,
            target_name: "*".to_string(),
            kind: EdgeKind::Imports,
            line,
            module: module_path.clone(),
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            col: 0,
        });
    }
}
