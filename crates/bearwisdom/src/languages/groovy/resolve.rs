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

pub const DGM_NAMESPACE: &str = "groovy.runtime.DefaultGroovyMethods";

/// Groovy language resolver.
///
/// Delegates all resolution logic to `JavaResolver` (same JVM scoping rules)
/// and adds a Groovy-specific external-namespace check for DGM methods.
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
        let target = &ref_ctx.extracted_ref.target_name;

        // DGM / GDK / Groovy DSL methods are mixed onto every object at
        // runtime — they have no declaration in user code, so classify them
        // as external immediately to keep them out of unresolved_refs.
        // Only fires for bare EdgeKind::Calls (the extractor emits no chain).
        if ref_ctx.extracted_ref.kind == EdgeKind::Calls
            && predicates::is_groovy_builtin(target)
        {
            return Some(DGM_NAMESPACE.to_string());
        }

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
    fn dgm_namespace_constant_is_stable() {
        assert_eq!(DGM_NAMESPACE, "groovy.runtime.DefaultGroovyMethods");
    }

    #[test]
    fn groovy_resolver_declares_only_groovy_language() {
        let r = GroovyResolver;
        assert_eq!(r.language_ids(), &["groovy"]);
    }

    #[test]
    fn dgm_call_infers_external_namespace() {
        let file_ctx = groovy_file_ctx();
        let sym = dummy_sym();

        for method in &[
            "each", "find", "findAll", "collect", "any", "sort",
            "push", "pop", "addAll", "join", "stripIndent", "stripMargin",
            "with", "tap", "flatten", "inject", "groupBy",
            "eachWithIndex", "collectEntries", "unique", "first", "last",
        ] {
            let r = ExtractedRef {
                source_symbol_index: 0,
                target_name: method.to_string(),
                kind: EdgeKind::Calls,
                line: 1,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
};
            let ref_ctx = RefContext {
                extracted_ref: &r,
                source_symbol: &sym,
                scope_chain: vec!["org.example.Foo".to_string()],
                file_package_id: None,
            };

            let ns = GroovyResolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
            assert_eq!(
                ns.as_deref(),
                Some(DGM_NAMESPACE),
                "expected DGM namespace for `{method}`"
            );
        }
    }

    #[test]
    fn non_dgm_call_does_not_infer_dgm_namespace() {
        let file_ctx = groovy_file_ctx();
        let sym = dummy_sym();

        let r = ExtractedRef {
            source_symbol_index: 0,
            target_name: "processOrder".to_string(),
            kind: EdgeKind::Calls,
            line: 1,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
};
        let ref_ctx = RefContext {
            extracted_ref: &r,
            source_symbol: &sym,
            scope_chain: vec!["org.example.Foo".to_string()],
            file_package_id: None,
        };

        let ns = GroovyResolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
        assert!(ns.is_none(), "project method should not be classified as external");
    }

    #[test]
    fn dgm_classification_requires_calls_edge() {
        let file_ctx = groovy_file_ctx();
        let sym = dummy_sym();

        let r = ExtractedRef {
            source_symbol_index: 0,
            target_name: "each".to_string(),
            kind: EdgeKind::Imports,
            line: 1,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
};
        let ref_ctx = RefContext {
            extracted_ref: &r,
            source_symbol: &sym,
            scope_chain: vec![],
            file_package_id: None,
        };

        let ns = GroovyResolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
        assert!(ns.is_none(), "non-Calls edge for 'each' should not hit DGM path");
    }
}
