// =============================================================================
// astro/resolve_tests.rs
//
// End-to-end binding of `.astro` template component tags through the
// frontmatter import block. The `---`-delimited frontmatter is sub-extracted
// as TypeScript; its component imports must reach `FileContext.imports` so the
// template `<Card />` tag binds to the project-defined component symbol.
// =============================================================================

use super::hooks::AstroHooks;
use super::profile::ASTRO_PROFILE;
use crate::indexer::resolve::legacy::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex, SymbolLookup,
};
use crate::languages::LanguagePlugin;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

fn run_resolve(
    file_ctx: &FileContext,
    ref_ctx: &RefContext<'_>,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    crate::type_checker::core::DefaultResolver {
        file_ctx,
        ref_ctx,
        lookup,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&ASTRO_PROFILE)
}

fn make_symbol(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 10,
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
    }
}

fn make_file(
    path: &str,
    language: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn build_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files
        .iter()
        .map(|f| make_file(&f.path, &f.language, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// Parse an `.astro` `source` as the indexer does: run the host extractor for
/// template component `Calls` refs, then sub-extract each emitted TypeScript
/// region (the `---` frontmatter, inline `<script>` blocks) and merge the
/// import refs into the host file's ref vector. The result mirrors
/// `parse_file`'s post-splice `ParsedFile`, so the frontmatter import bindings
/// land in `pf.refs` with their `module` field intact.
fn parse_astro(path: &str, source: &str) -> ParsedFile {
    let plugin = super::AstroPlugin;
    let host = plugin.extract(source, path, "astro");
    let mut symbols = host.symbols;
    let mut refs = host.refs;
    for region in plugin.embedded_regions(source, path, "astro") {
        if region.language_id != "typescript" {
            continue;
        }
        let sub = crate::languages::typescript::extract::extract(&region.text, false);
        let symbol_offset = symbols.len();
        for mut sr in sub.refs {
            sr.source_symbol_index += symbol_offset;
            refs.push(sr);
        }
        symbols.extend(sub.symbols);
    }
    make_file(path, "astro", symbols, refs)
}

#[test]
fn frontmatter_import_binds_template_component_tag() {
    // The canonical `.astro` shape: the `---` frontmatter imports a project
    // component, the template uses it as a tag. The frontmatter is TypeScript;
    // its import must reach `FileContext.imports` so the `<Card />` template
    // `Calls` ref binds the internal `Card` symbol.
    let card = make_file(
        "src/components/Card.astro",
        "astro",
        vec![make_symbol("Card", "Card", SymbolKind::Class)],
        vec![],
    );
    let page = parse_astro(
        "src/pages/index.astro",
        "---\nimport Card from '../components/Card.astro';\n---\n<main>\n  <Card title=\"Hi\" />\n</main>\n",
    );

    let (index, id_map) = build_env(&[&page, &card]);
    let file_ctx = AstroHooks.build_file_context(&page, None).unwrap();
    assert!(
        file_ctx.imports.iter().any(|e| e.imported_name == "Card"),
        "frontmatter import must populate FileContext.imports; got {:?}",
        file_ctx.imports
    );

    let call = page
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Card")
        .expect("host extractor must emit a Calls ref for <Card />");
    let source_symbol = &page.symbols[0];
    let ref_ctx = RefContext {
        extracted_ref: call,
        source_symbol,
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = run_resolve(&file_ctx, &ref_ctx, &index)
        .expect("imported <Card /> tag must bind the internal Card component");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("src/components/Card.astro".to_string(), "Card".to_string()))
            .unwrap()
    );
}

#[test]
fn template_tag_with_no_frontmatter_import_falls_through() {
    // No frontmatter import names the tag — the engine must not fabricate a
    // binding; the ref lands unresolved.
    let page = parse_astro(
        "src/pages/index.astro",
        "---\nconst x = 1;\n---\n<Unrelated />\n",
    );
    let (index, _) = build_env(&[&page]);
    let file_ctx = AstroHooks.build_file_context(&page, None).unwrap();
    let call = page
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Unrelated")
        .expect("host extractor must emit a Calls ref for <Unrelated />");
    let source_symbol = &page.symbols[0];
    let ref_ctx = RefContext {
        extracted_ref: call,
        source_symbol,
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    assert!(
        run_resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "no import → no Tier-1 resolution"
    );
}
