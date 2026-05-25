// Astro language hooks. `.astro` frontmatter (`---` block) and inline
// `<script>` blocks are TypeScript, so resolution delegates to the TS resolver
// — same approach as Vue/Svelte SFCs.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct AstroHooks;

impl LanguageEngineHooks for AstroHooks {
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

    /// Build the per-file import table for a `.astro` file. The frontmatter
    /// (sub-extracted as TypeScript) holds the component imports; delegate to
    /// the TS resolver so they populate `FileContext.imports`. Without this the
    /// engine's bare-name path has no imports for `.astro` files and every
    /// `<Component>` template ref is unresolved (astro sat at 0%).
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(
            crate::languages::typescript::hooks::TypeScriptResolver
                .build_file_context(file, project_ctx),
        )
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        crate::languages::typescript::hooks::TypeScriptResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static ASTRO_HOOKS: AstroHooks = AstroHooks;
