// =============================================================================
// languages/starlark/resolve.rs — Starlark / Bazel BUILD file resolution
//
// Starlark (used in Bazel BUILD files and .bzl extensions) references:
//
//   load("//tools/build_defs:foo.bzl", "my_rule")  → Imports
//   load("@bazel_skylib//lib:paths.bzl", "paths")  → Imports (external)
//   my_rule(name = "target", ...)                  → Calls, target_name = "my_rule"
//   cc_library(name = "lib", ...)                  → Calls (built-in Bazel rule)
//   native.cc_binary(...)                          → Calls (native namespace)
//
// Resolution strategy:
//   1. `load()` imports → collect the loaded symbols and their source files.
//   2. Same-file: functions/constants defined in the same .bzl file.
//   3. Import-based lookup: for each loaded symbol, check the source .bzl file.
//   4. Global name fallback.
//
// External namespace: `"bazel"` for native Bazel rules and built-in functions.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct StarlarkResolver;

impl LanguageResolver for StarlarkResolver {
    fn language_ids(&self) -> &[&str] {
        &["starlark", "bzl"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // load() statements import named symbols from a .bzl file.
            // The extractor emits one Imports ref per loaded symbol.
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone(),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "starlark".to_string(),
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

        // Import declarations themselves don't resolve to a symbol.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Bazel native rules and Starlark built-ins are external.
        if builtins::is_starlark_builtin(target) {
            return None;
        }

        // Step 1: Same-file resolution (def, assignment).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "starlark_same_file",
                });
            }
        }

        // Step 2: Import-based resolution.
        // A loaded symbol `my_rule` from `//tools:foo.bzl` should resolve to
        // the `my_rule` function defined in `foo.bzl`.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            let Some(mod_path) = &import.module_path else {
                continue;
            };

            // Skip external (@repo) references — they're external packages.
            if mod_path.starts_with('@') {
                return None;
            }

            // Convert Bazel label to relative path: "//tools/build_defs:foo.bzl"
            // → "tools/build_defs/foo.bzl"
            let file_path = bazel_label_to_path(mod_path);
            for sym in lookup.in_file(&file_path) {
                if sym.name == *target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "starlark_load_import",
                    });
                }
            }

            // Try a global lookup for the loaded name.
            for sym in lookup.by_name(target) {
                if matches!(sym.kind.as_str(), "function" | "variable") {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "starlark_load_global",
                    });
                }
            }
        }

        // Step 3: Global name fallback.
        if let Some(sym) = lookup.by_name(target).into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.75,
                strategy: "starlark_global_fallback",
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // `native.*` attribute calls are always Bazel built-ins, regardless of
        // whether the specific method appears in the static enumeration.
        // Covers: native.cc_binary, native.cc_test, native.py_library, etc.
        if target == "native" || target.starts_with("native.") {
            return Some("bazel_native".to_string());
        }

        if builtins::is_starlark_builtin(target) {
            return Some("bazel".to_string());
        }

        // load() from external repositories (@bazel_skylib, @rules_*) are external.
        // This applies to both the module-label ref and each loaded symbol ref,
        // since extract_load_refs propagates the module path to all.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or("");
            if module.starts_with('@') {
                return Some("bazel".to_string());
            }
        }

        None
    }
}

/// Convert a Bazel label to a file path.
/// "//tools/build_defs:foo.bzl" → "tools/build_defs/foo.bzl"
/// "//tools/build_defs/foo.bzl" → "tools/build_defs/foo.bzl"
fn bazel_label_to_path(label: &str) -> String {
    let label = label.trim_start_matches("//");
    // Replace ":" with "/" to convert package:target to a path.
    label.replacen(':', "/", 1)
}
