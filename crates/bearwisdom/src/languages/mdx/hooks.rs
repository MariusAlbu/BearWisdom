// MDX hooks. Absorbed from the deleted `mdx/resolve.rs`. MDX dispatches by
// ref kind: Imports go through the markdown link resolver; everything else
// goes through TypeScript via the TS resolver.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct MdxHooks;

impl LanguageEngineHooks for MdxHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }
        crate::languages::typescript::resolve::infer_external_inner_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        )
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(
            crate::languages::typescript::resolve::build_file_context_inner(
                file, project_ctx,
            ),
        )
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return crate::languages::markdown::hooks::resolve_markdown_link(
                file_ctx, ref_ctx, lookup,
            );
        }
        crate::languages::typescript::resolve::TypeScriptResolver.resolve(
            file_ctx, ref_ctx, lookup,
        )
    }
}

pub static MDX_HOOKS: MdxHooks = MdxHooks;
