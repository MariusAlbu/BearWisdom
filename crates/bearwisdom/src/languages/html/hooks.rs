//! HTML language hooks.
//!
//! Plain HTML has no resolver of its own, but a `<script src="./app.js">` tag
//! host-links the page to a project JS/TS file: the inline `<script>` blocks
//! call functions defined there. Those inline-script calls are extracted as
//! cross-language embedded JS `Calls` refs; the generic JS ladder can't bind a
//! bare call to a whole-file include because the `<script src>` import names a
//! FILE, not a member.
//!
//! `build_file_context` exposes the `<script src>` references as import entries
//! (so the host file context is non-empty and reaches the host-hook fallback),
//! and `resolve_ref` binds a bare embedded call to the unique project symbol
//! defined in a referenced script file. CDN / absolute `src` values are dropped
//! at extraction time, so only relative, in-project links reach here.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct HtmlHooks;

/// Join a relative `<script src>` URL onto the host HTML file's directory and
/// normalize `.`/`..` segments, returning a `/`-separated project-relative
/// path. Query/hash suffixes are dropped. Returns `None` for an empty or
/// root-relative (`/…`) URL — those have no host-directory anchor here.
fn resolve_relative_script(host_file: &str, url: &str) -> Option<String> {
    let clean = url.split(&['?', '#'][..]).next()?.trim();
    if clean.is_empty() || clean.starts_with('/') {
        return None;
    }
    let host_norm = host_file.replace('\\', "/");
    let mut segs: Vec<&str> = match host_norm.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').filter(|s| !s.is_empty()).collect(),
        None => Vec::new(),
    };
    for part in clean.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            other => segs.push(other),
        }
    }
    Some(segs.join("/"))
}

/// Bind a bare cross-language embedded `Calls` ref (an inline-`<script>` call)
/// to the unique project symbol of that name defined in a `<script src>`-linked
/// file. Returns the resolution, or `None` when no script link names the
/// defining file or the name is ambiguous / external.
fn resolve_via_script_src(
    ref_ctx: &RefContext<'_>,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let r = ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return None;
    }
    let target = r.target_name.as_str();
    if target.is_empty() || target.contains('.') {
        return None;
    }

    // Project-relative paths each `<script src>` resolves to, against this
    // HTML file's directory.
    let linked: Vec<String> = file_ctx
        .imports
        .iter()
        .filter_map(|i| i.module_path.as_deref())
        .filter_map(|url| resolve_relative_script(&file_ctx.file_path, url))
        .collect();
    if linked.is_empty() {
        return None;
    }

    let mut chosen: Option<i64> = None;
    for sym in lookup.by_name(target) {
        if lookup.is_external_file(&sym.file_path) {
            continue;
        }
        let path = sym.file_path.replace('\\', "/");
        let path_stem = path.rsplit_once('.').map(|(s, _)| s).unwrap_or(&path);
        let linked_here = linked.iter().any(|l| *l == path || *l == path_stem);
        if !linked_here {
            continue;
        }
        match chosen {
            None => chosen = Some(sym.id),
            Some(id) if id != sym.id => return None,
            Some(_) => {}
        }
    }

    chosen.map(|id| Resolution {
        target_symbol_id: id,
        confidence: RESOLVED_CONFIDENCE,
        strategy: "html_script_src",
        resolved_yield_type: None,
        flow_emit: None,
    })
}

impl LanguageEngineHooks for HtmlHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let imports = file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| {
                let module = r.module.clone().unwrap_or_else(|| r.target_name.clone());
                ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module),
                    alias: None,
                    is_wildcard: false,
                }
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "html".to_string(),
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
        resolve_via_script_src(ref_ctx, file_ctx, lookup)
    }
}

pub static HTML_HOOKS: HtmlHooks = HtmlHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
