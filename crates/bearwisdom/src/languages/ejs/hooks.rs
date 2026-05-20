// EJS language hooks. Absorbed from the deleted `ejs/resolve.rs`.

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct EjsHooks;

pub(crate) fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(8);
    let base = lexical_normalize(&source_dir.join(target));
    let already_has_ext = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "ejs" | "html" | "htm"))
        .unwrap_or(false);
    out.push(base.clone());
    if !already_has_ext {
        let base_str = base.to_string_lossy().to_string();
        out.push(PathBuf::from(format!("{base_str}.ejs")));
        out.push(PathBuf::from(format!("{base_str}.html")));
        out.push(base.join("index.ejs"));
    }
    out
}

fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method" | "constructor" | "class"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

pub(crate) fn lexical_normalize(p: &Path) -> PathBuf {
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

impl LanguageEngineHooks for EjsHooks {
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
                module_path: None,
                alias: None,
                is_wildcard: false,
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "ejs".to_string(),
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
            let source_dir = Path::new(&file_ctx.file_path).parent()?;
            for candidate in path_candidates(source_dir, target) {
                let path_str = candidate.to_string_lossy().replace('\\', "/");
                let stem = candidate
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                for sym in lookup.in_file(&path_str) {
                    if sym.kind == "class" && sym.name == stem {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "ejs_partial",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            return None;
        }
        engine::resolve_common("ejs", file_ctx, ref_ctx, lookup, kind_compatible)
    }
}

pub static EJS_HOOKS: EjsHooks = EjsHooks;
