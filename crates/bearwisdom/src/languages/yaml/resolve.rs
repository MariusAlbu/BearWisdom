// =============================================================================
// languages/yaml/resolve.rs — GitHub Actions `uses:` resolution
//
// The YAML host extractor emits one Imports ref per local `uses: ./...`
// directive. The shared resolver doesn't do path-based file lookup, so
// without this resolver every local action / reusable workflow reference
// lands in unresolved_refs.
//
// Resolution model mirrors HandlebarsResolver / EjsResolver:
//   1. Resolve the `uses:` target relative to the source file's parent.
//   2. Probe candidate file paths covering both GHA shapes:
//        - Reusable workflow: target ends with `.yml` / `.yaml` —
//          target IS the file.
//        - Composite/local action: target points at a directory whose
//          `action.yml` (or `action.yaml`) is the entry point.
//   3. Match against the host class symbol the YAML extractor emits per
//      file (`SymbolKind::Class` named after the file basename).
// =============================================================================

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

pub struct YamlResolver;

impl LanguageResolver for YamlResolver {
    fn language_ids(&self) -> &[&str] {
        &["yaml"]
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
            language: "yaml".to_string(),
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
            // The YAML extractor names its file-class symbol with the
            // full basename including extension (`ci.yml` not `ci`),
            // matching what `bw_file_symbols` shows for these files.
            let basename = candidate
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            for sym in lookup.in_file(&path_str) {
                if sym.kind == "class" && sym.name == basename {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "yaml_uses",
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

/// Generate candidate file paths for a GitHub Actions `uses:` reference.
///
/// Two shapes:
///   * Reusable workflow — `./.github/workflows/foo.yml`. Target IS
///     the file, no extension probing needed.
///   * Composite / JavaScript / Docker action — `./.github/actions/setup`.
///     Target is a directory whose `action.yml` or `action.yaml` is the
///     entry point.
fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(4);
    let base = lexical_normalize(&source_dir.join(target));

    let target_has_yaml_ext = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "yml" | "yaml"))
        .unwrap_or(false);

    if target_has_yaml_ext {
        // Reusable-workflow shape. The bare candidate is the target.
        out.push(base);
    } else {
        // Composite-action shape. Probe `action.yml` / `action.yaml`
        // inside the target directory.
        out.push(base.join("action.yml"));
        out.push(base.join("action.yaml"));
        // Defensive fallback: some local helpers ship as `<name>.yml`
        // alongside the workflows folder rather than as a directory
        // with `action.yml` inside.
        let bs = base.to_string_lossy().to_string();
        out.push(PathBuf::from(format!("{bs}.yml")));
        out.push(PathBuf::from(format!("{bs}.yaml")));
    }
    out
}

/// Resolve `..` / `.` components in a path lexically (no I/O).
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
