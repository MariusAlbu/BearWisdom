// SCSS language hooks. Absorbed from the deleted `scss/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ScssHooks;

/// The underscore-stripped, extensionless basename of a SCSS path. A partial
/// `_mixins.scss` and the `@use 'mixins'` that pulls it in both reduce to
/// `mixins`, so the include site can be matched to the defining partial by
/// stem regardless of the leading-underscore partial convention or a trailing
/// `.scss`/`.sass`/`.css` extension.
fn partial_stem(path: &str) -> &str {
    let basename = path.rsplit(&['/', '\\'][..]).next().unwrap_or(path);
    let stem = basename
        .strip_suffix(".scss")
        .or_else(|| basename.strip_suffix(".sass"))
        .or_else(|| basename.strip_suffix(".css"))
        .unwrap_or(basename);
    stem.strip_prefix('_').unwrap_or(stem)
}

/// Bind a bare `@include mixin` / `@function`-call ref to the SCSS mixin or
/// function symbol it names, when that symbol is in-index and reachable from
/// this file.
///
/// A `@mixin`/`@function` is extracted as a `Function` symbol; `@include name`
/// and a project-defined `name(...)` call are `Calls` refs. Same-file mixin
/// includes already bind on the generic same-file rung; this hook covers the
/// two cases that rung misses:
///   * a function call carries the synthesized CSS-function `module` hint that
///     declines before the generic ladder, so even a same-file `@function` call
///     never reaches `same_file`;
///   * a cross-partial include/call whose mixin lives in a `@use`d/`@import`ed
///     partial — the import names the FILE, not the member, so no generic rung
///     binds it.
///
/// Reachability is the file's own scope or an imported partial matched by
/// `partial_stem`. Binds only the UNIQUE in-project `Function` candidate; zero
/// or multiple decline so a genuine CSS built-in (`rgb`, `calc`) with no
/// project symbol, or an ambiguous name, stays unresolved.
fn resolve_via_scss_partial(
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

    let import_stems: Vec<&str> = file_ctx
        .imports
        .iter()
        .filter_map(|i| i.module_path.as_deref())
        .map(partial_stem)
        .collect();

    let mut chosen: Option<i64> = None;
    for sym in lookup.by_name(target) {
        if sym.kind != "function" || lookup.is_external_file(&sym.file_path) {
            continue;
        }
        let reachable = &*sym.file_path == file_ctx.file_path.as_str()
            || import_stems.contains(&partial_stem(&sym.file_path));
        if !reachable {
            continue;
        }
        match chosen {
            None => chosen = Some(sym.id),
            // A second distinct candidate is genuine ambiguity — decline.
            Some(id) if id != sym.id => return None,
            Some(_) => {}
        }
    }

    chosen.map(|id| Resolution {
        target_symbol_id: id,
        confidence: RESOLVED_CONFIDENCE,
        strategy: "scss_partial_include",
        resolved_yield_type: None,
        flow_emit: None,
    })
}

/// A ref's `module` names a non-project provider the binder must never resolve
/// through: a Sass built-in module (`@use 'sass:math'`) or the synthesized
/// CSS-function-call hint. Drives the profile's `module_skip` so a module-
/// carrying ref declines before the ladder and external classification brands
/// it. The target-keyed `builtin_skip` sibling, keyed on `r.module`.
pub(crate) fn is_scss_skippable_module(module: &str) -> bool {
    module == super::extract::SCSS_CSS_FN_HINT || predicates::is_sass_builtin_module(module)
}

impl LanguageEngineHooks for ScssHooks {
    fn resolve_bare_post(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        resolve_via_scss_partial(ref_ctx, file_ctx, lookup)
    }

    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if module == super::extract::SCSS_CSS_FN_HINT {
                return Some("css".to_string());
            }
        }
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if predicates::is_sass_builtin_module(path) {
                return Some(path.to_string());
            }
        }
        for import in &file_ctx.imports {
            let alias_matches =
                import.alias.as_deref() == Some(target.as_str()) || import.imported_name == *target;
            if !alias_matches {
                continue;
            }
            if let Some(mp) = &import.module_path {
                if !mp.starts_with('.') && !mp.starts_with('/') {
                    let pkg = mp.split('/').next().unwrap_or(mp.as_str());
                    return Some(pkg.to_string());
                }
            }
        }
        None
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let bare_segment = module_path
                .rsplit('/')
                .next()
                .unwrap_or(module_path.as_str())
                .trim_start_matches('_')
                .trim_end_matches(".scss")
                .trim_end_matches(".sass")
                .trim_end_matches(".css");
            let alias = if r.target_name != bare_segment {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "scss".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static SCSS_HOOKS: ScssHooks = ScssHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
