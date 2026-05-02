// =============================================================================
// languages/ejs/resolve.rs — EJS partial-include resolution
//
// The EJS host extractor emits one Imports ref per `<%- include('path') %>`
// call. The shared resolver doesn't do path-based file lookup, so without
// a language-specific resolver every include lands in unresolved_refs.
//
// Resolution model mirrors HandlebarsResolver / MarkdownResolver:
//   1. Resolve the include target relative to the source file's parent.
//   2. Probe candidate paths covering EJS conventions:
//        - Bare path
//        - With `.ejs` / `.html` extension when the original has none
//        - Directory `index.ejs` when the path resolves to a directory
//   3. Match against the host class symbol the EJS extractor emits per
//      file (`SymbolKind::Class` named after the file stem).
// =============================================================================

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

pub struct EjsResolver;

impl LanguageResolver for EjsResolver {
    fn language_ids(&self) -> &[&str] {
        &["ejs"]
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
            language: "ejs".to_string(),
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
                        strategy: "ejs_partial",
                        resolved_yield_type: None,
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

/// Generate candidate file paths for an EJS include reference.
///
/// EJS convention is `include('./partials/header')` — relative to the
/// source file's directory, with the `.ejs` extension implied. Some
/// templates use `include('partials/header')` without the leading `./`;
/// both forms resolve identically.
///
/// Probes (in order):
///   - Bare candidate (handles paths that already include `.ejs`)
///   - With `.ejs` appended when no extension present
///   - With `.html` appended (some setups render plain HTML partials)
///   - Directory-style: `<target>/index.ejs`
fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
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

/// Resolve `..` / `.` components in a path lexically (no I/O). Output
/// uses forward slashes via callers stringifying with `replace('\\', "/")`.
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
