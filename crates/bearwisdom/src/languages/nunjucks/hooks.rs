// =============================================================================
// nunjucks/hooks.rs — Nunjucks engine hooks.
//
// Nunjucks (`.njk`) is a Jinja2-compatible template DSL.
//   * `{% extends "base.njk" %}` and `{% include "partial.njk" %}` extract as
//     Imports refs whose `target_name` carries the template path.
//   * `{{ expr }}` interpolation dispatches to JavaScript via embedded
//     regions; those refs carry `ref_origin_lang = "javascript"` and resolve
//     through the JS hook, not this one.
//
// The DefaultResolver tower handles in-file macro lookups and any qname-
// based refs the JS embed might surface.
// =============================================================================

use std::path::{Component, Path, PathBuf};

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct NunjucksHooks;

fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(4);
    let base = lexical_normalize(&source_dir.join(target));
    let already_has_ext = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "njk" | "nunjucks" | "html" | "htm"))
        .unwrap_or(false);
    out.push(base.clone());
    if !already_has_ext {
        let base_str = base.to_string_lossy().to_string();
        out.push(PathBuf::from(format!("{base_str}.njk")));
        out.push(PathBuf::from(format!("{base_str}.nunjucks")));
        out.push(PathBuf::from(format!("{base_str}.html")));
    }
    out
}

impl LanguageEngineHooks for NunjucksHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let imports: Vec<ImportEntry> = file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "nunjucks".to_string(),
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
        let target = ref_ctx.extracted_ref.target_name.trim();
        if target.is_empty() {
            return None;
        }
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let Some(source_dir) = Path::new(&file_ctx.file_path).parent() else {
                return None;
            };
            for candidate in path_candidates(source_dir, target) {
                let path_str = candidate.to_string_lossy().replace('\\', "/");
                let stem = candidate.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                for sym in lookup.in_file(&path_str) {
                    if sym.kind == "class" && sym.name == stem {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "nunjucks_partial",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            return None;
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static NUNJUCKS_HOOKS: NunjucksHooks = NunjucksHooks;
