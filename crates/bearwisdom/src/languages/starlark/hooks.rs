// Starlark / Bazel BUILD file hooks. Only the data-inexpressible seams remain:
// external classification of Bazel framework chains / runtime-type method tails,
// and the file-context import table built from `load()` module labels. Bare-name,
// chain, and module-anchored binding run through the generic engine + profile data.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolLookup,
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
}

pub static STARLARK_HOOKS: StarlarkHooks = StarlarkHooks;
