// =============================================================================
// indexer/external_parse_payload.rs — serializable projection of a ParsedFile
//
// Converts a freshly-parsed external file to and from the persisted cache
// form. The contract is field-for-field fidelity: a rehydrated `ParsedFile`
// must carry the same resolution-relevant data as the fresh parse that
// produced it (same type surface, same snippet flags, same chains), so a warm
// reindex resolves identically to a cold one. Arena-specific `TypeId` /
// `GenericParamId` carriers cross the run boundary through the structural
// forms in `external_parse_types`.
// =============================================================================

use serde::{Deserialize, Serialize};

use super::external_parse_types::{CachedGenericParam, CachedType, TypeExporter, TypeImporter};
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    AliasTarget, CallArg, ChainSegment, EdgeKind, ExtractedRef, ExtractedRoute, ExtractedSymbol,
    FlowMeta, MemberChain, ParsedFile, SegmentKind, SymbolKind, Visibility,
};

#[cfg(test)]
#[path = "external_parse_payload_tests.rs"]
mod tests;

#[derive(Serialize, Deserialize)]
pub(crate) struct CachedSym {
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    visibility: Option<Visibility>,
    start_line: u32,
    end_line: u32,
    start_col: u32,
    end_col: u32,
    byte_offset: u32,
    signature: Option<String>,
    doc_comment: Option<String>,
    scope_path: Option<String>,
    parent_index: Option<usize>,
    declared_type: Option<CachedType>,
    return_type: Option<CachedType>,
    param_types: Vec<CachedType>,
    /// Indices into `CachedParse::type_params`.
    generic_params: Vec<usize>,
}

impl CachedSym {
    fn from_extracted(s: &ExtractedSymbol, ex: &mut TypeExporter<'_>) -> Self {
        CachedSym {
            name: s.name.clone(),
            qualified_name: s.qualified_name.clone(),
            kind: s.kind,
            visibility: s.visibility,
            start_line: s.start_line,
            end_line: s.end_line,
            start_col: s.start_col,
            end_col: s.end_col,
            byte_offset: s.byte_offset,
            signature: s.signature.clone(),
            doc_comment: s.doc_comment.clone(),
            scope_path: s.scope_path.clone(),
            parent_index: s.parent_index,
            declared_type: s.declared_type.map(|id| ex.export(id)),
            return_type: s.return_type.map(|id| ex.export(id)),
            param_types: s.param_types.iter().map(|&id| ex.export(id)).collect(),
            generic_params: s.generic_params.iter().map(|&g| ex.export_param(g)).collect(),
        }
    }

    fn into_extracted(self, im: &mut TypeImporter<'_>) -> ExtractedSymbol {
        ExtractedSymbol {
            name: self.name,
            qualified_name: self.qualified_name,
            kind: self.kind,
            visibility: self.visibility,
            start_line: self.start_line,
            end_line: self.end_line,
            start_col: self.start_col,
            end_col: self.end_col,
            byte_offset: self.byte_offset,
            signature: self.signature,
            doc_comment: self.doc_comment,
            scope_path: self.scope_path,
            parent_index: self.parent_index,
            declared_type: self.declared_type.as_ref().map(|t| im.import(t)),
            return_type: self.return_type.as_ref().map(|t| im.import(t)),
            param_types: self.param_types.iter().map(|t| im.import(t)).collect(),
            generic_params: self.generic_params.iter().map(|&i| im.import_param(i)).collect(),
        }
    }
}

/// Mirror of `ChainSegment` with the arena-specific `TypeId` carriers replaced
/// by their portable structural forms.
#[derive(Serialize, Deserialize)]
pub(crate) struct CachedSegment {
    name: String,
    node_kind: String,
    kind: SegmentKind,
    declared_type: Option<String>,
    type_args: Vec<String>,
    optional_chaining: bool,
    byte_offset: u32,
    declared_type_id: Option<CachedType>,
    type_arg_ids: Vec<CachedType>,
    is_call: bool,
    call_args: Vec<CallArg>,
}

impl CachedSegment {
    fn from_segment(s: &ChainSegment, ex: &mut TypeExporter<'_>) -> Self {
        CachedSegment {
            name: s.name.clone(),
            node_kind: s.node_kind.clone(),
            kind: s.kind,
            declared_type: s.declared_type.clone(),
            type_args: s.type_args.clone(),
            optional_chaining: s.optional_chaining,
            byte_offset: s.byte_offset,
            declared_type_id: s.declared_type_id.map(|id| ex.export(id)),
            type_arg_ids: s.type_arg_ids.iter().map(|&id| ex.export(id)).collect(),
            is_call: s.is_call,
            call_args: s.call_args.clone(),
        }
    }

    fn into_segment(self, im: &mut TypeImporter<'_>) -> ChainSegment {
        ChainSegment {
            name: self.name,
            node_kind: self.node_kind,
            kind: self.kind,
            declared_type: self.declared_type,
            type_args: self.type_args,
            optional_chaining: self.optional_chaining,
            byte_offset: self.byte_offset,
            declared_type_id: self.declared_type_id.as_ref().map(|t| im.import(t)),
            type_arg_ids: self.type_arg_ids.iter().map(|t| im.import(t)).collect(),
            is_call: self.is_call,
            call_args: self.call_args,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CachedRef {
    source_symbol_index: usize,
    target_name: String,
    kind: EdgeKind,
    line: u32,
    col: u32,
    byte_offset: u32,
    module: Option<String>,
    namespace_segments: Vec<String>,
    chain: Option<Vec<CachedSegment>>,
    call_args: Vec<CallArg>,
    is_import_binding: bool,
    is_reexport: bool,
    #[serde(default)]
    is_include: bool,
}

impl CachedRef {
    fn from_extracted(r: &ExtractedRef, ex: &mut TypeExporter<'_>) -> Self {
        CachedRef {
            source_symbol_index: r.source_symbol_index,
            target_name: r.target_name.clone(),
            kind: r.kind,
            line: r.line,
            col: r.col,
            byte_offset: r.byte_offset,
            module: r.module.clone(),
            namespace_segments: r.namespace_segments.clone(),
            chain: r.chain.as_ref().map(|c| {
                c.segments
                    .iter()
                    .map(|s| CachedSegment::from_segment(s, ex))
                    .collect()
            }),
            call_args: r.call_args.clone(),
            is_import_binding: r.is_import_binding,
            is_reexport: r.is_reexport,
            is_include: r.is_include,
        }
    }

    fn into_extracted(self, im: &mut TypeImporter<'_>) -> ExtractedRef {
        ExtractedRef {
            is_include: self.is_include,
            source_symbol_index: self.source_symbol_index,
            target_name: self.target_name,
            kind: self.kind,
            line: self.line,
            col: self.col,
            byte_offset: self.byte_offset,
            module: self.module,
            namespace_segments: self.namespace_segments,
            chain: self.chain.map(|segs| MemberChain {
                segments: segs.into_iter().map(|s| s.into_segment(im)).collect(),
            }),
            call_args: self.call_args,
            is_import_binding: self.is_import_binding,
            is_reexport: self.is_reexport,
        }
    }
}

/// The persisted extraction. Everything the external ingest/write surface
/// reads from a `ParsedFile` is here; the remaining fields are either derived
/// from the caller's context at rehydration time (`path`, `content_hash`,
/// `size`, `mtime`) or intentionally absent because no external-file consumer
/// reads them (`content` — re-read from disk where needed — `flow`, `db_sets`,
/// `demand_contributions`, `plugin_flow_emissions`).
#[derive(Serialize, Deserialize)]
pub(crate) struct CachedParse {
    language: String,
    package_id: Option<i64>,
    line_count: u32,
    has_errors: bool,
    symbols: Vec<CachedSym>,
    refs: Vec<CachedRef>,
    routes: Vec<ExtractedRoute>,
    symbol_origin_languages: Vec<Option<String>>,
    ref_origin_languages: Vec<Option<String>>,
    symbol_from_snippet: Vec<bool>,
    /// Type-alias targets, qualified by `ts_post_process_external` before the
    /// cache `put`. The chain walker reads these (intersection / mapped /
    /// typeof expansion) to resolve members reached THROUGH an external alias;
    /// dropping them on a cache hit silently breaks those chains while
    /// own-member lookup still works.
    alias_targets: Vec<(String, AliasTarget)>,
    /// Component selectors `(raw_selector, class_qname)`. Library `.d.ts`
    /// carry declaration selectors that back `selector_qname`; dropping them
    /// on a cache hit leaves every template ref to the component unresolved
    /// while the class symbol still loads.
    component_selectors: Vec<(String, String)>,
    /// File-local generic-parameter table referenced by `CachedType::Generic`
    /// and `CachedSym::generic_params`.
    type_params: Vec<CachedGenericParam>,
    /// Ambient `declare module '<name>'` names. The module-entry pass keys
    /// each name to the declaring file; dropping them on a cache hit would
    /// leave imports of those specifiers unlinked on warm reindexes.
    /// `default` keeps payloads written before the field readable.
    #[serde(default)]
    declared_modules: Vec<String>,
}

impl CachedParse {
    /// Project a freshly-parsed file into the persistable form, expanding
    /// every arena `TypeId` it references into a portable tree.
    pub(crate) fn from_parsed(pf: &ParsedFile, arena: &TypeArena) -> Self {
        let mut ex = TypeExporter::new(arena);
        let symbols = pf
            .symbols
            .iter()
            .map(|s| CachedSym::from_extracted(s, &mut ex))
            .collect();
        let refs = pf
            .refs
            .iter()
            .map(|r| CachedRef::from_extracted(r, &mut ex))
            .collect();
        CachedParse {
            language: pf.language.clone(),
            package_id: pf.package_id,
            line_count: pf.line_count,
            has_errors: pf.has_errors,
            symbols,
            refs,
            routes: pf.routes.clone(),
            symbol_origin_languages: pf.symbol_origin_languages.clone(),
            ref_origin_languages: pf.ref_origin_languages.clone(),
            symbol_from_snippet: pf.symbol_from_snippet.clone(),
            alias_targets: pf.alias_targets.clone(),
            component_selectors: pf.component_selectors.clone(),
            type_params: ex.into_params(),
            declared_modules: pf.declared_modules.clone(),
        }
    }

    /// Rebuild a `ParsedFile`, re-interning every cached type into `arena`.
    /// `path` / `content_hash` / `size` / `mtime` come from the caller — they
    /// are context, not part of the content-addressed payload.
    pub(crate) fn into_parsed(
        self,
        arena: &TypeArena,
        path: &str,
        content_hash: &str,
        size: u64,
        mtime: Option<i64>,
    ) -> ParsedFile {
        let mut im = TypeImporter::new(arena, self.type_params);
        ParsedFile {
            path: path.to_string(),
            language: self.language,
            content_hash: content_hash.to_string(),
            size,
            line_count: self.line_count,
            mtime,
            package_id: self.package_id,
            symbols: self
                .symbols
                .into_iter()
                .map(|s| s.into_extracted(&mut im))
                .collect(),
            refs: self
                .refs
                .into_iter()
                .map(|r| r.into_extracted(&mut im))
                .collect(),
            routes: self.routes,
            db_sets: Vec::new(),
            symbol_origin_languages: self.symbol_origin_languages,
            ref_origin_languages: self.ref_origin_languages,
            symbol_from_snippet: self.symbol_from_snippet,
            content: None,
            has_errors: self.has_errors,
            flow: FlowMeta::default(),
            demand_contributions: Vec::new(),
            alias_targets: self.alias_targets,
            component_selectors: self.component_selectors,
            plugin_flow_emissions: Vec::new(),
            declared_modules: self.declared_modules,
        }
    }
}
