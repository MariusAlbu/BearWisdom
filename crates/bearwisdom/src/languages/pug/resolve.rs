// =============================================================================
// languages/pug/resolve.rs — Pug `include` / `extends` resolution
//
// The host extractor emits one Imports ref per `include foo` or `extends
// layout` directive. The shared resolution engine doesn't do path-based
// file lookup, so without this resolver every `include includes/block`
// from `views/blockchain.pug` lands in unresolved_refs — even though the
// target file `views/includes/block.pug` is indexed.
//
// Resolution model mirrors HandlebarsResolver:
//   1. Resolve target relative to the source file's parent directory.
//   2. Try `<target>.pug`, `<target>.jade`, and the directory variant
//      `<target>/index.pug` to cover both common Pug layouts.
//   3. Match the file's host Class symbol (the file-stem symbol the Pug
//      extractor emits per file).
// =============================================================================

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

pub struct PugResolver;

impl LanguageResolver for PugResolver {
    fn language_ids(&self) -> &[&str] {
        &["pug", "jade"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
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
        FileContext {
            file_path: file.path.clone(),
            language: "pug".to_string(),
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
        if ref_ctx.extracted_ref.kind != EdgeKind::Imports {
            return None;
        }
        let target = ref_ctx.extracted_ref.target_name.trim();
        if target.is_empty() {
            return None;
        }
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
                        strategy: "pug_template_include",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        None
    }
}

/// Generate candidate file paths for a Pug `include` / `extends` target.
///
/// The extractor already strips the `.pug` / `.jade` extension and any
/// leading `./`, so `target` is something like `layout`, `includes/block`,
/// or `../shared/header`. Probe (in order):
///   1. `<source_dir>/<target>.pug`
///   2. `<source_dir>/<target>.jade`
///   3. `<source_dir>/<target>/index.pug` and `.../index.jade`
fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
    let base = lexical_normalize(&source_dir.join(target));
    let mut out: Vec<PathBuf> = Vec::with_capacity(4);
    let already_pug = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "pug" | "jade"))
        .unwrap_or(false);
    if already_pug {
        out.push(base.clone());
    } else {
        let base_str = base.to_string_lossy().to_string();
        out.push(PathBuf::from(format!("{base_str}.pug")));
        out.push(PathBuf::from(format!("{base_str}.jade")));
        out.push(PathBuf::from(format!("{base_str}/index.pug")));
        out.push(PathBuf::from(format!("{base_str}/index.jade")));
    }
    out
}

/// Lexical `..` collapse without touching the filesystem — `a/b/../c`
/// → `a/c`. Used so include candidates in indexed paths share the same
/// canonical form as the file rows we look up.
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

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
