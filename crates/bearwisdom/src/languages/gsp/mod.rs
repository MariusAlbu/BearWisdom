//! Grails Server Pages (.gsp). `${expr}` → Groovy expression.
//! `<g:render template="_x">` → Imports ref.
pub(crate) mod profile;
pub(crate) mod taglib;
pub use profile::GSP_PROFILE;

#[cfg(test)]
#[path = "embedded_tests.rs"]
mod embedded_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{
    EdgeKind, EmbeddedOrigin, EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};

pub struct GspPlugin;

impl LanguagePlugin for GspPlugin {
    fn id(&self) -> &str {
        "gsp"
    }
    fn language_ids(&self) -> &[&str] {
        &["gsp"]
    }
    fn extensions(&self) -> &[&str] {
        &[".gsp"]
    }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> {
        None
    }
    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }
    fn extract(&self, source: &str, file_path: &str, _l: &str) -> ExtractionResult {
        let norm = file_path.replace('\\', "/");
        let name = norm.rsplit('/').next().unwrap_or(&norm);
        let stem = std::path::Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name)
            .to_string();
        let symbols = vec![ExtractedSymbol {
            name: stem.clone(),
            qualified_name: stem,
            kind: SymbolKind::Class,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        }];
        let mut refs: Vec<ExtractedRef> = Vec::new();
        let line_starts: Vec<u32> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| (i + 1) as u32))
            .collect();

        // Namespaced markup taglib invocations (`<warehouse:message ...>`,
        // `<g:link ...>`). Each emits a `Calls` ref to the tag local name so a
        // custom taglib binds to its indexed closure definition; a Grails core
        // tag has no in-index target and is branded `grails-taglib` by the GSP
        // hook's external classifier. The `<g:render template="...">` template
        // edge is emitted separately below as an `Imports` ref.
        for tag in taglib::scan_markup_tags(source) {
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: 0,
                target_name: tag.name,
                kind: EdgeKind::Calls,
                line: tag.line as u32,
                module: None,
                chain: None,
                col: 0,
                byte_offset: tag.byte_offset as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }

        for (line_no, line) in source.lines().enumerate() {
            if let Some(pos) = line.find("<g:render") {
                let rest = &line[pos..];
                if let Some(idx) = rest.find("template=\"") {
                    let start = idx + 10;
                    if let Some(end) = rest[start..].find('"') {
                        let name = rest[start..start + end].trim_start_matches('_').to_string();
                        refs.push(ExtractedRef {
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: 0,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line: line_no as u32,
                            module: None,
                            chain: None,
                            col: 0,
                            byte_offset: line_starts.get(line_no).copied().unwrap_or(0),
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
            }
        }
        ExtractionResult {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: false,
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        let mut regions = crate::languages::common::extract_html_script_style_regions(source);
        let bytes = source.as_bytes();
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                let start = i + 2;
                let mut d = 1;
                let mut j = start;
                while j < bytes.len() && d > 0 {
                    match bytes[j] {
                        b'{' => d += 1,
                        b'}' => d -= 1,
                        _ => {}
                    }
                    if d == 0 {
                        break;
                    }
                    j += 1;
                }
                if j < bytes.len() && d == 0 {
                    if let Some(t) = source.get(start..j) {
                        let t = t.trim();
                        if !t.is_empty() {
                            let (line, col) = lc(bytes, start);
                            regions.push(EmbeddedRegion {
                                language_id: "groovy".into(),
                                text: format!("def x = ({t})\n"),
                                line_offset: line,
                                col_offset: col,
                                origin: EmbeddedOrigin::TemplateExpr,
                                holes: Vec::new(),
                                strip_scope_prefix: None,
                            });
                        }
                    }
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
        regions
    }
    fn symbol_node_kinds(&self) -> &[&str] {
        &[]
    }
    fn ref_node_kinds(&self) -> &[&str] {
        &[]
    }
    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::GSP_PROFILE)
    }
}

fn lc(bytes: &[u8], pos: usize) -> (u32, u32) {
    let mut line: u32 = 0;
    let mut nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(pos) {
        if *b == b'\n' {
            line += 1;
            nl = i + 1;
        }
    }
    (line, (pos - nl) as u32)
}
