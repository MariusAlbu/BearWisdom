// Groovy hooks — DGM/GDK methods classify via the engine's keywords()
// set; Java/JVM types come from the Java external classifier.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
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
}

pub static GROOVY_HOOKS: GroovyHooks = GroovyHooks;
