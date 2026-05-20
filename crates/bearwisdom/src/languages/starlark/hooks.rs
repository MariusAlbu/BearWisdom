// Starlark / Bazel BUILD file hooks. Absorbed from the deleted
// `starlark/resolve.rs`.

use super::{chain, predicates};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ExtractedRef, ParsedFile};

pub struct StarlarkHooks;

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

fn bazel_label_to_path(label: &str) -> String {
    let label = label.trim_start_matches("//");
    label.replacen(':', "/", 1)
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    _project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let full_name = dotted_name(&ref_ctx.extracted_ref);
    if full_name == "native" || full_name.starts_with("native.") {
        return Some("bazel_native".to_string());
    }
    if predicates::is_bazel_framework_chain(&full_name) {
        return Some("bazel".to_string());
    }
    if full_name.contains('.') && predicates::is_builtin_method_tail(&full_name) {
        return Some("starlark-runtime".to_string());
    }
    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or("");
        if module.starts_with('@') {
            return Some("bazel".to_string());
        }
    }
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

impl LanguageEngineHooks for StarlarkHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone(),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "starlark".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        let full_name = dotted_name(&ref_ctx.extracted_ref);
        if full_name.contains('.') && predicates::is_builtin_method_tail(&full_name) {
            return None;
        }
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
        }
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
        if predicates::is_bazel_framework_chain(&full_name) {
            return None;
        }
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
            if mod_path.starts_with('@') {
                return None;
            }
            let file_path = bazel_label_to_path(mod_path);
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

pub static STARLARK_HOOKS: StarlarkHooks = StarlarkHooks;
