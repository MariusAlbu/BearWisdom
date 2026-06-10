// MDX hooks. MDX dispatches by ref kind purely through profile data: `Imports`
// (markdown links) bind via `MDX_PROFILE.import_resolution` (`resolve_via_import_path`);
// JSX component refs flow through the TypeScript-shaped strategy ladder. The
// hook keeps only the external classifier and the import-table builder.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct MdxHooks;

impl LanguageEngineHooks for MdxHooks {
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

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(crate::languages::typescript::hooks::build_file_context_inner(file, project_ctx))
    }
}

pub static MDX_HOOKS: MdxHooks = MdxHooks;
