// =============================================================================
// indexer/resolve/rules/typescript.rs — TypeScript/JavaScript resolution rules
//
// Scope rules for TypeScript and JavaScript (ES2015+ module system):
//
//   1. Import resolution: refs that carry a `module` field come from import
//      statements. If the module is a relative path (starts with "./", "../"),
//      look up the target symbol in that module's file.
//   2. Same-file resolution: symbols defined in the same file are visible at
//      module scope without any import.
//   3. Scope chain walk: innermost scope → outermost, try {scope}.{target}.
//   4. Fully qualified: dotted names resolve directly.
//
// Key differences from C#:
//   - The TS/JS extractor emits import bindings as `EdgeKind::TypeRef` refs
//     (NOT `EdgeKind::Imports`) with the `module` field set to the raw import
//     specifier string (e.g., `"./utils"`, `"react"`).
//   - Bare specifiers (no "./" prefix) are external packages/builtins.
//   - No file-level namespace — `file_namespace` is always `None`.
//   - `build_file_context` collects import entries from any ref that has
//     a `module` field set (i.e., came from an import statement).
//
// Adding new TS features:
//   - New import syntax → update the extractor (parser/extractors/typescript.rs)
//     to emit the ref with the `module` field set; this resolver picks it up.
//   - New scope forms → update scope_path in the extractor; the scope chain
//     walk handles them automatically.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use tracing::debug;

/// TypeScript and JavaScript language resolver.
pub struct TypeScriptResolver;

impl LanguageResolver for TypeScriptResolver {
    fn language_ids(&self) -> &[&str] {
        &["typescript", "javascript", "tsx", "jsx"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Collect import entries from any ref that has a `module` field set.
        //
        // The TS/JS parser emits one ref per imported binding, e.g.:
        //   import { useState, useEffect } from 'react'
        //     → ref { target_name: "useState",  module: "react",  kind: TypeRef }
        //     → ref { target_name: "useEffect", module: "react",  kind: TypeRef }
        //
        //   import React from 'react'           (default import)
        //     → ref { target_name: "React",     module: "react",  kind: TypeRef }
        //
        //   import { formatDate } from './utils'
        //     → ref { target_name: "formatDate", module: "./utils", kind: TypeRef }
        //
        // We distinguish external (bare) vs relative by the module specifier.
        // is_wildcard is unused in the TS resolver — all TS imports are explicit.
        for r in &file.refs {
            let Some(module_path) = r.module.clone() else {
                continue;
            };
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }

        // TypeScript has no file-level namespace — module identity is the file path.
        FileContext {
            file_path: file.path.clone(),
            language: file.language.clone(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip EdgeKind::Imports — TS/JS extractor rarely emits these, but be safe.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // If the ref itself carries a module path, it came from an import statement.
        // Attempt to resolve the symbol in the source module.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            // External packages are not in our index — skip.
            if is_bare_specifier(module) {
                return None;
            }

            // Relative import: look up in the target file by simple name.
            for sym in lookup.in_file(module) {
                if sym.name == *target && kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_import_file",
                        file = %module,
                        target = %target,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_import_file",
                    });
                }
            }

            // Also try {module}.{target} as a qualified name (parser may use this form).
            let candidate = format!("{module}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_import",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_import",
                    });
                }
            }

            // Import ref but couldn't resolve — fall back to heuristic.
            return None;
        }

        // No module on the ref — this is a non-import reference.

        // Normalize: strip `this.` prefix for member access on the current class.
        // `this.buildUserRO` → `buildUserRO`, then scope chain resolves it.
        // `this.db.selectFrom` → `db.selectFrom` (still a chain, handled later).
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["MyClass.method", "MyClass"]
        // Try "MyClass.method.target", "MyClass.target"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_scope_chain",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        // In TS/JS, symbols in the same file are visible at module scope.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_same_file",
                    qualified_name = %sym.qualified_name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_same_file",
                });
            }
        }

        // Step 3: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "ts_qualified_name",
                        target = %target,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ts_qualified_name",
                    });
                }
            }
        }

        // Step 4: Field type chain resolution.
        // For `db.selectFrom` (after stripping `this.`), split into field + rest,
        // find the field's type annotation, then look up the method on that type.
        if let Some(dot) = effective_target.find('.') {
            let field_name = &effective_target[..dot];
            let rest = &effective_target[dot + 1..];

            // Try to find the field as a property on enclosing scopes.
            for scope in &ref_ctx.scope_chain {
                let field_qname = format!("{scope}.{field_name}");
                if let Some(type_name) = lookup.field_type_name(&field_qname) {
                    // Found field type. Try {TypeName}.{rest} in the index.
                    let candidate = format!("{type_name}.{rest}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.95,
                                strategy: "ts_field_type_chain",
                            });
                        }
                    }

                    // Also try: the type might be in a namespace, search by name.
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    for sym in lookup.by_name(method_name) {
                        if sym.qualified_name.starts_with(type_name)
                            && kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "ts_field_type_chain",
                            });
                        }
                    }

                    // Type is known but method isn't in our index — it's on the type.
                    // Don't fall through; let infer_external_namespace handle it.
                    break;
                }
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Browser/JS runtime globals — always external.
        if is_js_runtime_global(target) {
            return Some("runtime".to_string());
        }

        // If the ref itself carries a module path, check it directly.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_bare_specifier(module) {
                let is_external = match project_ctx {
                    Some(ctx) => ctx.is_external_ts_package(module),
                    // Without ProjectContext, treat all bare specifiers as external.
                    None => true,
                };
                if is_external {
                    return Some(module.clone());
                }
            }
            // Relative import with a module — not external.
            return None;
        }

        // No module on the ref — check the file's import list for this target.
        // If the name was imported from a bare specifier, it's external.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            let Some(module_path) = &import.module_path else {
                continue;
            };
            if !is_bare_specifier(module_path) {
                continue;
            }
            let is_external = match project_ctx {
                Some(ctx) => ctx.is_external_ts_package(module_path),
                None => true,
            };
            if is_external {
                return Some(module_path.clone());
            }
        }

        None
    }

    // is_visible: default implementation (always true) is correct for TS.
    // TypeScript's `export` keyword controls visibility, but for resolution
    // purposes we treat all indexed symbols as accessible.
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A bare specifier is a module path that does not start with ".", "/", or a
/// drive letter — i.e., it refers to an npm package or Node.js built-in.
///
/// Examples:
///   - `"react"` → bare (external)
///   - `"@tanstack/react-query"` → bare (external)
///   - `"node:fs"` → bare (external)
///   - `"./utils"` → relative (internal)
///   - `"../shared/types"` → relative (internal)
///   - `"/absolute/path"` → absolute (internal)
pub fn is_bare_specifier(s: &str) -> bool {
    !s.starts_with('.')
        && !s.starts_with('/')
        // Windows absolute paths (e.g. "C:/...")
        && !(s.len() >= 2 && s.as_bytes()[1] == b':')
}

/// Detect references to browser/JS runtime globals.
///
/// Matches the object prefix for dotted names like `document.querySelector`,
/// `JSON.stringify`, `console.error`, `Promise.all`, etc.
/// Also matches standalone globals like `setTimeout`, `encodeURIComponent`.
fn is_js_runtime_global(target: &str) -> bool {
    // Extract the object (first segment) for dotted names.
    let obj = target.split('.').next().unwrap_or(target);
    matches!(
        obj,
        // DOM / Browser APIs
        "document" | "window" | "navigator" | "location" | "history"
            | "localStorage" | "sessionStorage" | "performance"
            // Global objects
            | "console" | "JSON" | "Math" | "Object" | "Array"
            | "Promise" | "RegExp" | "Date" | "Map" | "Set"
            | "WeakMap" | "WeakSet" | "Symbol" | "Proxy" | "Reflect"
            | "Error" | "TypeError" | "RangeError" | "SyntaxError"
            | "Intl" | "Number" | "String" | "Boolean"
            // Global functions
            | "setTimeout" | "setInterval" | "clearTimeout" | "clearInterval"
            | "requestAnimationFrame" | "cancelAnimationFrame"
            | "fetch" | "atob" | "btoa"
            | "encodeURIComponent" | "decodeURIComponent"
            | "encodeURI" | "decodeURI"
            | "parseInt" | "parseFloat" | "isNaN" | "isFinite"
            | "structuredClone" | "queueMicrotask"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
///
/// TypeScript is structurally typed and more permissive than C# — we allow
/// more combinations here.
fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "interface" | "type_alias"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class"
                | "interface"
                | "enum"
                | "type_alias"
                | "function"
                | "variable"
                | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "typescript_tests.rs"]
mod tests;
