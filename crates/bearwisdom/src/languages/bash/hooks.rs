// Bash language hooks. Absorbed from the deleted `bash/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct BashHooks;

pub(crate) fn shell_path_suffix(raw: &str) -> &str {
    if raw.starts_with('/') {
        return "";
    }
    if raw.starts_with('$') {
        if let Some(slash) = raw.find('/') {
            return &raw[slash + 1..];
        }
        return "";
    }
    raw.trim_start_matches("./").trim_start_matches("../")
}

pub(crate) fn ends_with_path_suffix(file_path: &str, suffix: &str) -> bool {
    if file_path == suffix {
        return true;
    }
    if file_path.ends_with(suffix) {
        let prefix_len = file_path.len() - suffix.len();
        let boundary = file_path
            .as_bytes()
            .get(prefix_len.saturating_sub(1))
            .copied();
        matches!(boundary, Some(b'/' | b'\\'))
    } else {
        false
    }
}

fn resolve_via_shell_source(
    target_name: &str,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let candidates = lookup.by_name(target_name);
    if candidates.is_empty() {
        return None;
    }
    for import in &file_ctx.imports {
        let raw_path = match &import.alias {
            Some(p) => p.as_str(),
            None => continue,
        };
        if !raw_path.ends_with(".sh") && !raw_path.ends_with(".bash") {
            continue;
        }
        let suffix = shell_path_suffix(raw_path);
        if suffix.is_empty() {
            continue;
        }
        for sym in candidates {
            if ends_with_path_suffix(&sym.file_path, suffix) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.90,
                    strategy: "bash_shell_source",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
}

impl LanguageEngineHooks for BashHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_bash_builtin)
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
            let raw_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: Some(raw_path),
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "shell".to_string(),
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
        if predicates::is_bash_builtin(target) {
            return None;
        }
        if let Some(res) = engine::resolve_common(
            "bash",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        ) {
            return Some(res);
        }
        if edge_kind == EdgeKind::Calls {
            if let Some(res) = resolve_via_shell_source(target, file_ctx, lookup) {
                return Some(res);
            }
        }
        if edge_kind == EdgeKind::Calls && ref_ctx.extracted_ref.module.is_none() {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_bash = path.ends_with(".sh")
                    || path.ends_with(".bash")
                    || path.starts_with("ext:bash-completion-synthetics:");
                if !is_bash {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "bash_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        None
    }
}

pub static BASH_HOOKS: BashHooks = BashHooks;
