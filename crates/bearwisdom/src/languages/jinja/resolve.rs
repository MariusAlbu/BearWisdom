//! Jinja2 reference resolver.
//!
//! Two ref kinds to handle:
//!
//! 1. **TypeRef** — identifier-chain heads emitted by the expression scanner
//!    (`{{ user.name }}` → `user`). Resolves via the engine's common path,
//!    which handles same-file Variable symbols (loop vars from `{% for %}`,
//!    set bindings from `{% set %}`, macro params).
//!
//! 2. **Imports** — `{% extends "base.j2" %}`, `{% include "partials/x.j2" %}`,
//!    `{% import "macros.j2" as m %}`. The target_name is a relative file
//!    stem; same shape as Markdown link refs. We mirror MarkdownResolver's
//!    candidate-walk: resolve relative to the source file's directory,
//!    try each Jinja extension, bind to the file's host Class symbol.

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

pub struct JinjaResolver;

impl LanguageResolver for JinjaResolver {
    fn language_ids(&self) -> &[&str] {
        &["jinja", "jinja2", "j2"]
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
            language: "jinja".to_string(),
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
        match ref_ctx.extracted_ref.kind {
            EdgeKind::Imports => resolve_template_path(file_ctx, ref_ctx, lookup),
            _ => engine::resolve_common("jinja", file_ctx, ref_ctx, lookup, kind_compatible),
        }
    }
}

fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        // `{{ name }}` references match Variables (loop/set/macro bindings),
        // Fields (block declarations), and the host Class symbol for
        // self-references / `{% extends self %}` patterns.
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "variable" | "field" | "class" | "function" | "type_alias" | "parameter"
        ),
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        _ => true,
    }
}

/// Resolve `{% extends/include/import "path" %}` refs by walking the relative
/// target against the source file's directory and binding to whichever
/// candidate file has a host symbol indexed.
fn resolve_template_path(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let target = &ref_ctx.extracted_ref.target_name;
    if target.is_empty() {
        return None;
    }
    let source_dir = Path::new(&file_ctx.file_path).parent()?;
    let raw = source_dir.join(target);
    let normalized = lexical_normalize(&raw);

    for candidate in candidate_paths(&normalized) {
        let candidate_str = candidate.to_string_lossy().replace('\\', "/");
        if let Some(host) = lookup
            .in_file(&candidate_str)
            .into_iter()
            .find(|s| s.kind == "class")
        {
            return Some(Resolution {
                target_symbol_id: host.id,
                confidence: 0.95,
                strategy: "jinja_template_path",
                resolved_yield_type: None,
            });
        }
    }
    None
}

fn candidate_paths(normalized: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    out.push(normalized.to_path_buf());
    for ext in &["j2", "jinja", "jinja2"] {
        let mut p = normalized.to_path_buf();
        let cur_ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if cur_ext.is_empty() {
            p.set_extension(ext);
            out.push(p);
        } else if cur_ext != *ext {
            // Already has an extension (e.g. `nginx.conf` from a config
            // template) — also try with the jinja extension appended.
            let mut combo = normalized.to_path_buf();
            let new_name = format!(
                "{}.{}",
                combo.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                ext
            );
            combo.set_file_name(new_name);
            out.push(combo);
        }
    }
    out
}

fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
