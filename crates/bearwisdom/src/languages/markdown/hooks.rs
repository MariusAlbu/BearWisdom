// Markdown language hooks. Absorbed from the deleted `markdown/resolve.rs`.

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

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        resolve_markdown_link(file_ctx, ref_ctx, lookup)
    }
}

pub static MARKDOWN_HOOKS: MarkdownHooks = MarkdownHooks;
