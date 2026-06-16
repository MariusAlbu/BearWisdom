// Groovy hooks. Absorbed from the deleted `groovy/resolve.rs`. Groovy reuses
// Java's resolver and external classifier; GORM detection layers on top.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, RefContext, SymbolLookup};
use crate::languages::java::hooks::build_file_context_inner as java_build_file_context;
use crate::languages::java::hooks::infer_external_inner as java_infer;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{MemberChain, ParsedFile};

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
    if !root
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
    {
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

/// A bare Groovy `Calls` ref hosted in a GSP file that names a standard Grails
/// core tag. GSP `${...}` expressions are sub-parsed as Groovy, so a tag called
/// in expression scope (`${message(code:'x')}`) surfaces as a receiver-less
/// Groovy call whose target is the tag name. When the Grails framework sources
/// are not materialized on disk these names have no in-index target; the finite
/// standard-tag contract brands them as a framework builtin. Gated three ways so
/// a same-named project method in a plain `.groovy` file is never declined:
/// the host must be a GSP file, the call must be receiver-less, and the target
/// must be in the standard contract.
pub(crate) fn gsp_standard_tag(
    ref_ctx: &RefContext<'_>,
    file_ctx: &FileContext,
) -> Option<String> {
    if !file_ctx.file_path.ends_with(".gsp") {
        return None;
    }
    let r = ref_ctx.extracted_ref;
    if r.kind != crate::types::EdgeKind::Calls || r.chain.is_some() {
        return None;
    }
    if crate::languages::gsp::taglib::is_standard_grails_tag(&r.target_name) {
        Some("grails-taglib".to_string())
    } else {
        None
    }
}

impl LanguageEngineHooks for GroovyHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if let Some(ns) = gsp_standard_tag(ref_ctx, file_ctx) {
            return Some(ns);
        }
        java_infer(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let mut emissions = crate::languages::java::hooks::detect_flow_inner(file_ctx, ref_ctx);
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
        Some(java_build_file_context(file, project_ctx))
    }
}

pub static GROOVY_HOOKS: GroovyHooks = GroovyHooks;
