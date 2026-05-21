// =============================================================================
// languages/javascript/hooks.rs — engine hook delegating to TypeScriptResolver
//
// JS and TS share the runtime, the lib.*.d.ts surface, the module resolver,
// and the import shapes. The TS resolver is content-driven (reads imports,
// scope chain, qnames) and has no TypeScript-only syntactic dependency, so
// it is the right engine for JS files too — including the JS sub-extraction
// inside Vue 2 SFCs whose `<script>` blocks default to JavaScript.
//
// Wiring this hook closes the gap where embedded-JS refs in `.vue` files
// (origin_language = "javascript") never reached the TS resolver's
// `ts_lib_globals` fallback, leaving `parseInt`, `setTimeout`, `XMLHttpRequest`,
// `Array.prototype.splice`, and friends unresolved.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::languages::typescript::hooks::{
    build_file_context_inner, infer_external_inner_with_lookup, TypeScriptResolver,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct JavascriptHooks;

impl LanguageEngineHooks for JavascriptHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner_with_lookup(file_ctx, ref_ctx, project_ctx, lookup)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        TypeScriptResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static JAVASCRIPT_HOOKS: JavascriptHooks = JavascriptHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
