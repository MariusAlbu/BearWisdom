// =============================================================================
// groovy/resolve.rs — Groovy language resolver
//
// Wraps JavaResolver (shared JVM resolution rules) and adds a Groovy-specific
// bare-name step that also searches symbols from `.groovy` source files.
//
// Without the extra step: bare calls on static methods of same-package classes
// (e.g. `isAndroidProject()` calling `Utils.isAndroidProject(Project)`) miss
// `java_bare_name` because that step's file-extension filter excludes `.groovy`
// to prevent cross-language collisions.  The extra `groovy_bare_name` step
// applies the same visibility guards but accepts `.groovy` paths in addition to
// the `.java`/`.jar` paths the Java resolver already handles.
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
/// Delegates all resolution logic to `JavaResolver` (same JVM scoping rules),
/// then adds a `groovy_bare_name` step that finds same-package methods in
/// `.groovy` source files when the Java bare-name step has no match.
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
        // Primary path: full Java resolver chain (scope walk, same-package,
        // imports, FQN, inheritance walk, java_bare_name).
        if let Some(res) = JavaResolver.resolve(file_ctx, ref_ctx, lookup) {
            return Some(res);
        }

        // Groovy-specific extension: bare-name lookup across `.groovy` files.
        //
        // `java_bare_name` skips `.groovy` paths to avoid cross-language
        // collisions with identically-named Python/TS symbols.  That filter is
        // safe for Java but too strict for Groovy: same-project static helpers
        // (e.g. `Utils.isAndroidProject`) live in `.groovy` files and must be
        // reachable from bare calls in other `.groovy` files.
        let edge_kind = ref_ctx.extracted_ref.kind;
        let target = &ref_ctx.extracted_ref.target_name;
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && ref_ctx.extracted_ref.chain.is_none()
            && !effective_target.contains('.')
        {
            for sym in lookup.by_name(effective_target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if !sym.file_path.ends_with(".groovy") {
                    continue;
                }
                if !self.is_visible(file_ctx, ref_ctx, sym) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "groovy_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        None
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        target: &crate::indexer::resolve::engine::SymbolInfo,
    ) -> bool {
        JavaResolver.is_visible(file_ctx, ref_ctx, target)
    }

    fn detect_flow_emission(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        // Reuse Java detectors — Groovy/Grails uses the same annotations
        // (`@GetMapping`, etc.), JPA, JdbcTemplate, etc.
        let mut emissions = JavaResolver.detect_flow_emission(file_ctx, ref_ctx);
        // GORM `User.where { ... }.list()`, `User.findById(id)` — uses chain.
        if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
            if let Some(em) = detect_groovy_gorm_emission(chain) {
                emissions.push(em);
            }
        }
        emissions
    }
}

pub(crate) fn detect_groovy_gorm_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    let leaf = chain.segments.last()?.name.as_str();
    if !root.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
        return None;
    }
    let op = match leaf {
        "list" | "findAll" | "findAllBy" | "findBy" | "findById" | "get" | "where" | "count"
        | "first" | "last" => DbQueryOp::Select,
        "save" | "insert" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" | "deleteAll" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("groovy.{}", root),
        operation: op,
    })
}

