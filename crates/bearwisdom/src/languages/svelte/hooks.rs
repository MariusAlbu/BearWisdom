use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct SvelteHooks;

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
    /// `<script>` block IS TypeScript, so delegate to `SvelteResolver`
    /// (which wraps the TS resolver). Without this the engine falls back to
    /// the legacy path that never populates `FileContext.imports`, so
    /// `import X from '$lib/...'` is invisible to the import-based strategies.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(super::SvelteResolver.build_file_context(file, project_ctx))
    }

    /// Resolve a `.svelte` ref via the TS-backed `SvelteResolver`, then fall
    /// through to the generic resolver tower with TS kind-compatibility — so
    /// `$lib`/path-alias imports of template components resolve through
    /// `resolve_via_aliased_import`.
    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if let Some(res) = super::SvelteResolver.resolve(file_ctx, ref_ctx, lookup) {
            return Some(res);
        }
        (crate::type_checker::core::DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: crate::languages::typescript::predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static SVELTE_HOOKS: SvelteHooks = SvelteHooks;
