// Astro language hooks. `.astro` frontmatter (`---` block) and inline
// `<script>` blocks are TypeScript, so resolution delegates to the TS resolver
// — same approach as Vue/Svelte SFCs.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
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
            file_ctx,
            ref_ctx,
            project_ctx,
            lookup,
        )
    }

    /// Build the per-file import table for a `.astro` file. The frontmatter
    /// (sub-extracted as TypeScript) holds the component imports; build the
    /// table through the shared TS file-context builder so they populate
    /// `FileContext.imports`. Without imports the engine's bare-name path can't
    /// bind a `<Component>` template ref.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(crate::languages::typescript::hooks::build_file_context_inner(file, project_ctx))
    }
}

pub static ASTRO_HOOKS: AstroHooks = AstroHooks;
