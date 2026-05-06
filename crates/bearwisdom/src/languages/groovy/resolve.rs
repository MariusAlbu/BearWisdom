// =============================================================================
// groovy/resolve.rs — Groovy language resolver
//
// Wraps JavaResolver (shared JVM resolution rules) and adds a Groovy-specific
// layer in `infer_external_namespace` that classifies DefaultGroovyMethods
// (DGM), GDK additions, and Groovy DSL builtins as external refs instead of
// unresolved refs.
//
// Why we need this: The Groovy extractor emits bare calls (chain: None) for
// every method_invocation — it does not yet build MemberChain segments. This
// means the chain walker is never invoked for Groovy, and DGM methods like
// `each`, `find`, `collect`, `push`, etc. have no symbol in any project scope
// to resolve to. Without a Groovy-specific external classifier they all land
// in unresolved_refs as noise.
//
// The fix: override `infer_external_namespace` to check `is_groovy_builtin`
// first. If the bare call name is a known DGM / GDK / DSL method, we classify
// it under the synthetic namespace "groovy.runtime.DefaultGroovyMethods" so it
// lands in external_refs and disappears from unresolved noise.
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use crate::languages::java::resolve::JavaResolver;
use super::predicates;

/// Groovy language resolver.
///
/// Delegates all resolution logic to `JavaResolver` (same JVM scoping rules).
/// DGM/GDK Object-mixin methods classify via the engine's keywords() set
/// populated from groovy/keywords.rs.
pub struct GroovyResolver;

impl LanguageResolver for GroovyResolver {
    fn language_ids(&self) -> &[&str] {
        &["groovy"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        JavaResolver.build_file_context(file, project_ctx)
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        JavaResolver.resolve(file_ctx, ref_ctx, lookup)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // DGM / GDK Object-mixin methods (each, collect, with, tap, ...) are
        // classified by the engine's keywords() set populated from
        // groovy/keywords.rs. Java/JVM types come via JavaResolver below.
        // Delegate remaining classification to the Java resolver (import
        // namespace checks, ALWAYS_EXTERNAL prefixes, manifest deps, etc.).
        JavaResolver.infer_external_namespace(file_ctx, ref_ctx, project_ctx)
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        target: &crate::indexer::resolve::engine::SymbolInfo,
    ) -> bool {
        JavaResolver.is_visible(file_ctx, ref_ctx, target)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::resolve::engine::FileContext;
    use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};

    fn dummy_sym() -> ExtractedSymbol {
        ExtractedSymbol {
            name: "dummy".to_string(),
            qualified_name: "org.example.Foo.dummy".to_string(),
            kind: SymbolKind::Method,
            visibility: None,
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some("org.example.Foo".to_string()),
            parent_index: None,
        }
    }

    fn groovy_file_ctx() -> FileContext {
        FileContext {
            file_path: "src/Foo.groovy".to_string(),
            language: "groovy".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        }
    }

    #[test]
    fn groovy_resolver_declares_only_groovy_language() {
        let r = GroovyResolver;
        assert_eq!(r.language_ids(), &["groovy"]);
    }

    // DGM-classification tests removed when is_groovy_builtin was deleted.
    // DGM/GDK names now classify via the engine's keywords() set populated
    // from groovy/keywords.rs; the namespace string changed from
    // "groovy.dgm" to "primitive".
}
