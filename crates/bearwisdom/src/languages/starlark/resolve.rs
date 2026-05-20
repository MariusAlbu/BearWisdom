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

use super::{chain, predicates};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ExtractedRef, ParsedFile};

pub struct StarlarkResolver;

/// Reconstruct the full dotted call path from a ref.
///
/// When a ref carries a MemberChain, `target_name` is the last segment name
/// only (REF-004). The full dotted path needed for qualified-name lookups,
/// framework-chain predicate checks, and import-alias splitting is rebuilt
/// from the chain segments. Falls back to `target_name` when no chain is
/// present.
fn dotted_name(r: &ExtractedRef) -> String {
    if let Some(ch) = r.chain.as_ref() {
        ch.segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    } else {
        r.target_name.clone()
    }
}

impl StarlarkResolver {

    
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
pub(crate) fn resolve(
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

        // Bazel native rules / Starlark builtins / skylib helpers classify
        // via the engine's keywords() set populated from
        // starlark/keywords.rs — the upstream classification flow handles
        // them so we don't need a fast-exit here.

        // Reconstruct the full dotted path for predicates and lookups that
        // need to inspect the chain root or use the qualified name. For refs
        // with a MemberChain, `target_name` is the leaf only (REF-004).
        let full_name = dotted_name(&ref_ctx.extracted_ref);

        // Dotted call whose last segment is a Python/Starlark built-in
        // type method (str/list/dict/depset). `output.append`, `filename
        // .endswith`, `kwargs.pop` always bind to runtime types — return
        // None so infer_external_namespace classifies them as external.
        if full_name.contains('.') && predicates::is_builtin_method_tail(&full_name) {
            return None;
        }

        // Chain-walker resolution: for dotted refs (ctx.actions.run, etc.) try
        // a direct qualified-name lookup against the synthetic ctx API symbols
        // before routing to external classification. This produces real edges
        // (strategy "starlark_ctx_chain") instead of opaque external refs.
        if predicates::is_bazel_framework_chain(&full_name)
            || ref_ctx.extracted_ref.chain.is_some()
        {
            if let Some(res) = chain::resolve(
                ref_ctx.extracted_ref.chain.as_ref(),
                &full_name,
                edge_kind,
                Some(file_ctx),
                ref_ctx,
                lookup,
            ) {
                return Some(res);
            }
            // Chain walker missed — fall through so the predicate guards below
            // still route this ref to external (preserving round-1 behaviour).
        }

        // Bazel synthetic dotted lookup: refs like `target.runfiles`,
        // `runfiles.merge_all`, `args.add_joined`, `attr.label`, `config.exec`
        // have synthetic symbols emitted under their exact qualified-name in
        // ext:bazel-builtins:ctx.bzl. Try a direct by_qualified_name hit so
        // these resolve to real edges instead of staying unresolved.
        if full_name.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(&full_name) {
                if sym.file_path.starts_with("ext:bazel-builtins:")
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "starlark_bazel_synthetic",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Bazel framework parameter chains not resolved by the chain walker:
        // classify as external rather than leaving as unresolved.
        if predicates::is_bazel_framework_chain(&full_name) {
            return None;
        }

        // Step 1: Same-file resolution (def, assignment).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "starlark_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Step 2: Import-based resolution.
        // Handles both direct names (`my_rule`) and qualified calls (`unittest.begin`).
        //
        // For `load("//lib:unittest.bzl", "unittest")`:
        //   - Direct: target="unittest" → look up in loaded file
        //   - Qualified: target="unittest.begin" → the chain carries [unittest, begin];
        //     full_name = "unittest.begin"; split on "." to match import alias "unittest"
        //     and resolve "begin" in the loaded file.
        let (import_alias, member_name) = if full_name.contains('.') {
            let dot = full_name.find('.').unwrap();
            (&full_name[..dot], Some(&full_name[dot + 1..]))
        } else {
            (full_name.as_str(), None)
        };

        for import in &file_ctx.imports {
            if import.imported_name != import_alias {
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

            // If it's a qualified call (unittest.begin), resolve the member
            // within the loaded file.
            let resolve_name = member_name.unwrap_or(target.as_str());
            for sym in lookup.in_file(&file_path) {
                if sym.name == resolve_name {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "starlark_load_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Try a global lookup for the resolved name.
            for sym in lookup.by_name(resolve_name) {
                if matches!(sym.kind.as_str(), "function" | "variable") {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "starlark_load_global",
                        resolved_yield_type: None,
                        flow_emit: None,
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
                resolved_yield_type: None,
                flow_emit: None,
            });
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

pub(super) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    _project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    // Reconstruct the full dotted path for predicate checks that inspect
    // the chain root (native.*, ctx.*, repository_ctx.*, etc.).
    let full_name = dotted_name(&ref_ctx.extracted_ref);

    // `native.*` attribute calls are always Bazel built-ins, regardless of
    // whether the specific method appears in the static enumeration.
    // Covers: native.cc_binary, native.cc_test, native.py_library, etc.
    if full_name == "native" || full_name.starts_with("native.") {
        return Some("bazel_native".to_string());
    }

    // Bazel framework parameter roots: `ctx.*`, `repository_ctx.*`, `env.*`,
    // `directory.*` — these are opaque objects passed by the Bazel runtime.
    // Any dotted ref starting with one of these roots is external at any depth
    // (covers ctx.label.name, env.expect.that_str, directory.glob, etc.).
    if predicates::is_bazel_framework_chain(&full_name) {
        return Some("bazel".to_string());
    }

    // Dotted method call whose tail is a Python/Starlark builtin —
    // classify as runtime since it binds to an str/list/dict/etc. type.
    if full_name.contains('.') && predicates::is_builtin_method_tail(&full_name) {
        return Some("starlark-runtime".to_string());
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

    // Import walk: if the target (or its first dotted segment) was loaded
    // from an external @-repository, classify as external.
    // e.g., `asserts.equals` where `asserts` was loaded from `@bazel_skylib//...`
    let simple = full_name.split('.').next().unwrap_or(&full_name);
    for import in &file_ctx.imports {
        if import.imported_name != simple {
            continue;
        }
        if let Some(mod_path) = &import.module_path {
            if mod_path.starts_with('@') {
                return Some("bazel".to_string());
            }
        }
    }

    None
}

pub(crate) fn build_file_context_inner(
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
