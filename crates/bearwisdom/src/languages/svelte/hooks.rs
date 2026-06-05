use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct SvelteHooks;

/// Svelte 5 runes are compiler built-ins, not stores — `$state(0)` must not be
/// desugared to a reference to a symbol named `state`.
const SVELTE_RUNES: &[&str] = &[
    "state", "derived", "effect", "props", "bindable", "inspect", "host",
];

/// `$store` is Svelte sugar for subscribing to the store `store`; the
/// reference is semantically to `store`. Returns the underlying identifier
/// for a store-subscription ref, or `None` when the name is not one: a bare
/// `$`, the `$$props`/`$$restProps`/`$$slots` specials, a Svelte 5 rune, or a
/// name that is not a single bare identifier (e.g. carries a `.`).
pub(crate) fn svelte_store_base(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('$')?;
    if rest.is_empty() || rest.starts_with('$') {
        return None;
    }
    let mut chars = rest.chars();
    let first_ok = chars.next().map_or(false, |c| c.is_alphabetic() || c == '_');
    if !first_ok || !rest.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    if SVELTE_RUNES.contains(&rest) {
        return None;
    }
    Some(rest)
}

/// Rewrite a `$store` reference in place to its underlying identifier: both
/// the `target_name` and, for a chain like `$page.url`, the root segment.
/// No-op for non-store refs. Applied to a Svelte SFC's embedded `<script>`
/// refs at splice time so the chain walker and bare resolver both see the
/// store identifier the Svelte compiler subscribes to.
pub(crate) fn desugar_store_ref_in_place(rf: &mut crate::types::ExtractedRef) {
    if let Some(base) = svelte_store_base(&rf.target_name) {
        rf.target_name = base.to_string();
    }
    if let Some(chain) = rf.chain.as_mut() {
        if let Some(root) = chain.segments.first_mut() {
            if let Some(base) = svelte_store_base(&root.name) {
                root.name = base.to_string();
            }
        }
    }
}

impl LanguageEngineHooks for SvelteHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        crate::languages::typescript::hooks::infer_external_inner_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        )
    }

    /// Build the per-file import table for a `.svelte` file. Svelte's
    /// `<script>` block IS TypeScript, so the import table comes from the shared
    /// TS file-context builder. Without this the engine's generic fallback would
    /// still build a context, but routing through the TS builder keeps the
    /// NestJS-prefix / bgjob-queue synthetic entries consistent with `.ts` files.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(crate::languages::typescript::hooks::build_file_context_inner(
            file, project_ctx,
        ))
    }
}

pub static SVELTE_HOOKS: SvelteHooks = SvelteHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
