// Groovy hooks. Absorbed from the deleted `groovy/resolve.rs`. Groovy reuses
// Java's resolver and external classifier; GORM detection layers on top.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::languages::java::hooks::infer_external_inner as java_infer;
use crate::languages::java::hooks::JavaResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, MemberChain, ParsedFile};

pub struct GroovyHooks;

pub(crate) fn detect_groovy_gorm_emission(
    chain: &MemberChain,
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

impl LanguageEngineHooks for GroovyHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        java_infer(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let mut emissions =
            crate::languages::java::hooks::detect_flow_inner(file_ctx, ref_ctx);
        if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
            if let Some(em) = detect_groovy_gorm_emission(chain) {
                emissions.push(em);
            }
        }
        emissions
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(JavaResolver.build_file_context(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if let Some(res) = JavaResolver.resolve(file_ctx, ref_ctx, lookup) {
            return Some(res);
        }
        let edge_kind = ref_ctx.extracted_ref.kind;
        let target = &ref_ctx.extracted_ref.target_name;
        let effective_target = target.strip_prefix("this.").unwrap_or(target);
        if matches!(
            edge_kind,
            EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates
        ) && ref_ctx.extracted_ref.module.is_none()
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
                if !JavaResolver.is_visible(file_ctx, ref_ctx, sym) {
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
}

pub static GROOVY_HOOKS: GroovyHooks = GroovyHooks;
