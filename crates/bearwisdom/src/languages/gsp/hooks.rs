// GSP language hooks.
//
// The plugin keeps a hook only to build the per-file resolution context. GSP
// carries no symbol-level imports — `<g:render template="...">` resolution
// reads the ref's target name directly — so the import list is empty. Template
// binding itself is generic engine code driven by the profile's
// `import_resolution` data.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::indexer::resolve::legacy::types::RESOLVED_CONFIDENCE;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct GspHooks;

impl LanguageEngineHooks for GspHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "gsp".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        })
    }

    /// Bind a bare `Calls` ref in a `.gsp` file to a uniquely-named taglib
    /// `Method` symbol in the index. Declines on zero or multiple candidates
    /// so coincidentally-shared names never produce ambiguous edges.
    fn resolve_bare_post(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let r = ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls || r.chain.is_some() {
            return None;
        }
        if !file_ctx.file_path.ends_with(".gsp") {
            return None;
        }
        let candidates: Vec<_> = lookup
            .by_name(&r.target_name)
            .into_iter()
            .filter(|s| s.kind == "method")
            .collect();
        if candidates.len() == 1 {
            Some(Resolution {
                target_symbol_id: candidates[0].id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "gsp_taglib_method",
                resolved_yield_type: None,
                flow_emit: None,
            })
        } else {
            None
        }
    }

    /// Brand a `<ns:tag>` markup `Calls` ref that names a Grails core tag.
    /// A custom project taglib binds to its indexed closure definition through
    /// the normal ladder and never reaches this classifier; a framework tag
    /// (`g:link`, `g:if`, ...) has no in-index target, so the finite core-tag
    /// contract marks it a framework builtin rather than a miscounted
    /// unresolved ref. The receiver-less / Calls gate mirrors the bare-tag
    /// expression contract.
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let r = ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls || r.chain.is_some() {
            return None;
        }
        if super::taglib::is_grails_markup_tag(&r.target_name) {
            Some("grails-taglib".to_string())
        } else {
            None
        }
    }
}

pub static GSP_HOOKS: GspHooks = GspHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
