// Markdown language hooks.
//
// The plugin keeps a hook only to build the per-file resolution context (the
// `Imports` ref list with empty module paths). Markdown's OWN relative-link
// resolution is generic engine code driven by the profile's `import_resolution`
// data; there is no `resolve_ref` impl here.
//
// `resolve_markdown_link` (and the path helpers it needs) is retained as a
// shared resolver: the MDX plugin reuses it for the link-import half of its
// own ref dispatch (the other half routes through TypeScript). It is not wired
// into Markdown's own resolution path.

use std::path::{Component, Path, PathBuf};

use tracing::debug;

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct MarkdownHooks;

pub(crate) fn path_candidates(normalized: &Path) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(16);
    out.push(normalized.to_path_buf());
    let extensions = ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];
    let already_markdown = normalized
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| extensions.contains(&e))
        .unwrap_or(false);
    if !already_markdown {
        let base = normalized.to_string_lossy().to_string();
        for ext in extensions {
            out.push(PathBuf::from(format!("{base}.{ext}")));
        }
    }
    let entries = ["index", "README", "readme", "Readme"];
    for entry in entries {
        for ext in extensions {
            out.push(normalized.join(format!("{entry}.{ext}")));
        }
    }
    out
}

pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                let pop_ok = matches!(
                    stack.last(),
                    Some(Component::Normal(_)) | Some(Component::CurDir)
                );
                if pop_ok {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    stack.iter().collect()
}

/// Resolve a markdown relative link to the target file's stem-named class.
/// Shared with the MDX plugin, which reuses it for its link-import refs. Mirrors
/// the generic `resolve_via_import_path` behavior the `markdown` profile drives,
/// expressed directly here because MDX's ref dispatch needs to special-case the
/// `Imports` kind before falling through to its TypeScript resolution.
pub(crate) fn resolve_markdown_link(
    file_ctx: &FileContext,
    ref_ctx: &RefContext<'_>,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    if ref_ctx.extracted_ref.kind != EdgeKind::Imports {
        return None;
    }
    let target = &ref_ctx.extracted_ref.target_name;
    if target.is_empty() {
        return None;
    }
    let source_dir = Path::new(&file_ctx.file_path).parent()?;
    let raw_joined = source_dir.join(target);
    let normalized = lexical_normalize(&raw_joined);
    for candidate in path_candidates(&normalized) {
        let path_str = candidate.to_string_lossy().replace('\\', "/");
        let stem = candidate
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        for sym in lookup.in_file(&path_str) {
            if sym.kind == "class" && sym.name == stem {
                debug!(
                    strategy = "markdown_relative_link",
                    candidate = %path_str,
                    target = %target,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "markdown_relative_link",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
}

impl LanguageEngineHooks for MarkdownHooks {
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
            language: "markdown".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static MARKDOWN_HOOKS: MarkdownHooks = MarkdownHooks;
