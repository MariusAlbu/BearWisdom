// Groovy hooks — DGM/GDK methods classify via the engine's keywords()
// set; Java/JVM types come from the Java external classifier.

use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::languages::java::resolve::infer_external_inner as java_infer;
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct GroovyHooks;

impl LanguageEngineHooks for GroovyHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Groovy reuses Java's external classifier — same JVM ecosystem,
        // pom.xml/Gradle manifest data, same ALWAYS_EXTERNAL prefixes.
        java_infer(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        resolve::detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &crate::indexer::resolve::engine::FileContext,
        ref_ctx: &crate::indexer::resolve::engine::RefContext<'_>,
        lookup: &dyn crate::indexer::resolve::engine::SymbolLookup,
    ) -> Option<crate::indexer::resolve::engine::Resolution> {
        use crate::indexer::resolve::engine::LanguageResolver;
        super::resolve::GroovyResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static GROOVY_HOOKS: GroovyHooks = GroovyHooks;
