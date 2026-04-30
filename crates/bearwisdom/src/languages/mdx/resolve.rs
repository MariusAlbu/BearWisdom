// =============================================================================
// languages/mdx/resolve.rs — MDX cross-resolver
//
// MDX files mix two ref shapes in one host extraction:
//
//   1. Markdown-style relative-link `Imports` refs (extracted via
//      `markdown::host_scan::collect_link_refs`). Path-based — resolve
//      against the source file's parent dir, probe candidate files,
//      bind to the host class symbol of the target file.
//
//   2. JSX component `Calls` refs (extracted by `mdx::extract::collect_jsx_refs`).
//      The components are imported in MDX's top-level `import { Foo } from
//      'x'` statements, which `mdx::embedded` collects into a synthetic
//      TypeScript `ScriptBlock` region. The TS sub-extractor then emits
//      one TypeRef-with-module ref per import binding into the same
//      `ParsedFile.refs`, tagged via `ref_origin_languages` as
//      `typescript`.
//
// The resolution engine routes refs by their effective language:
//
//   - Spliced TS-import refs → `effective_lang = "typescript"` →
//     TypeScriptResolver. These resolve correctly today.
//   - Host JSX `Calls` refs → `effective_lang = pf.language = "mdx"` →
//     this resolver. We need to delegate to TypeScriptResolver so the
//     JSX component name matches the file's spliced TS imports.
//   - Host link `Imports` refs → same `"mdx"` routing → delegate to the
//     existing MarkdownResolver path-based logic.
//
// This mirrors the Vue/Svelte pattern (template tags resolved against
// the `<script>` block's TS imports) — same exact problem, same fix.
// Astro uses TypeScriptResolver directly because it has no link refs;
// MDX needs both halves so it gets a thin dispatcher.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::languages::markdown::resolve::MarkdownResolver;
use crate::languages::typescript::resolve::TypeScriptResolver;
use crate::types::{EdgeKind, ParsedFile};

pub struct MdxResolver;

impl LanguageResolver for MdxResolver {
    fn language_ids(&self) -> &[&str] {
        &["mdx"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // The TS sub-extractor's import refs carry `module` set, so the
        // TypeScript file-context builder picks them up directly. The
        // MDX host's link refs (kind = Imports, no module) are also
        // appended but bring no usable ImportEntry — that's fine; the
        // MarkdownResolver consults `extracted_ref.target_name` directly,
        // not file_ctx.imports.
        TypeScriptResolver.build_file_context(file, project_ctx)
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return MarkdownResolver.resolve(file_ctx, ref_ctx, lookup);
        }
        TypeScriptResolver.resolve(file_ctx, ref_ctx, lookup)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return MarkdownResolver.infer_external_namespace(file_ctx, ref_ctx, project_ctx);
        }
        TypeScriptResolver.infer_external_namespace(file_ctx, ref_ctx, project_ctx)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return MarkdownResolver.infer_external_namespace_with_lookup(
                file_ctx, ref_ctx, project_ctx, lookup,
            );
        }
        TypeScriptResolver.infer_external_namespace_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        )
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
