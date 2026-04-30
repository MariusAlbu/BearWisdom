// =============================================================================
// languages/markdown/resolve.rs — Markdown link resolution
//
// Markdown emits `EdgeKind::Imports` refs whose `target_name` is the path of
// a relative link, with the `.md`/`.markdown` extension stripped (see
// `host_scan.rs::collect_link_refs`):
//
//   `[arch](./architecture/overview.md)` → target_name "architecture/overview"
//   `[changelog](../CHANGELOG)`          → target_name "../CHANGELOG"
//
// The shared resolution engine is symbol-name oriented and never looks up
// by file path, so without this resolver every Markdown link lands in
// `unresolved_refs`. Real-world doc trees (dotnet-abp, dockerfile-
// nodebestpractices, fsharp-fstoolkit) carry thousands of these refs.
//
// Resolution model:
//
//   1. Resolve the relative target against the source file's parent dir.
//   2. Try the candidate as-is, then with each Markdown extension, then
//      as a directory containing `index.md` / `README.md`. Mirrors what
//      VitePress / Docusaurus / MkDocs do at render time.
//   3. For each candidate, query `SymbolLookup::in_file` for that path.
//      The Markdown host extractor emits a `Class` symbol per file named
//      after the file stem; bind the link to that host symbol so the
//      doc-graph view gets a real cross-file edge.
//
// Cross-language: an MDX or Astro link to a Markdown file resolves the
// same way — the target is a file path, the host symbol is a Class. The
// resolver lives in the markdown plugin because the link extraction does;
// MDX reuses it via `language_ids()`.
// =============================================================================

use std::path::{Component, Path, PathBuf};

use tracing::debug;

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

pub struct MarkdownResolver;

impl LanguageResolver for MarkdownResolver {
    fn language_ids(&self) -> &[&str] {
        // Markdown + MDX share the link-extraction shape (MDX's host scan
        // is a thin wrapper around Markdown's). One resolver covers both.
        &["markdown", "md", "mdx"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // Markdown has no per-file imports state to seed; the resolver
        // works directly off `extracted_ref.target_name`.
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
            language: "markdown".to_string(),
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
        // The only refs Markdown emits are link `Imports`. Anything else
        // is a coverage gap — bail.
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
                // The Markdown host extractor emits ONE Class symbol per
                // file, named after the file stem. That's the canonical
                // link target — symbols emitted from fenced code blocks
                // are intentionally separate (they have origin_language
                // set to the fence's language and aren't the doc anchor).
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
        // Markdown links that don't match an indexed file genuinely point
        // outside the project — anchor-only links and external URLs are
        // already filtered at extract time, so anything reaching this
        // resolver and failing path lookup is a real doc-drift signal.
        // Return None and let the ref land in unresolved_refs honestly
        // rather than synthesising an `ext:<path>` namespace that hides
        // broken doc links from the user.
        None
    }
}

/// Generate the ordered list of file-path candidates a Markdown link
/// could match. Mirrors the VitePress / Docusaurus / MkDocs convention:
///   `target` (as-is — link may already carry the extension)
///   `target.md`, `target.markdown`, `target.mdx`, …
///   `target/index.md`, `target/index.mdx`
///   `target/README.md`, `target/README.mdx`
///
/// Always append a Markdown extension as a separate candidate via
/// `with_added_extension`, NOT `with_extension` — `with_extension`
/// replaces an existing dotted suffix, which silently breaks paths
/// like `eslint_prettier.basque` (a translation suffix the file system
/// treats as `Path::extension == "basque"`). The translated file is
/// `eslint_prettier.basque.md`, so we want APPEND not REPLACE. Skip the
/// append when the path already ends in a known Markdown extension to
/// avoid emitting `foo.md.md` as a candidate.
fn path_candidates(normalized: &Path) -> Vec<PathBuf> {
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

/// Lexical path normalization that collapses `./` and `../` without
/// touching the filesystem. Mirrors std's unstable
/// `Path::normalize_lexically` so links resolve consistently regardless
/// of whether the referenced file exists at index time.
fn lexical_normalize(path: &Path) -> PathBuf {
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

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
