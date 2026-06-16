use super::hooks::MdxHooks;
use super::profile::MDX_PROFILE;
use crate::indexer::resolve::legacy::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

/// Drive an MDX ref through the generic engine exactly as the resolve loop does
/// for a profile-only language: a chain-bearing ref through the structured
/// walker, everything else through the `DefaultResolver` ladder gated on
/// `MDX_PROFILE` (so `Imports` refs hit `resolve_via_import_path` and JSX refs
/// hit the TypeScript-shaped strategies).
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
    .resolve_all_with_profile(&MDX_PROFILE)
}

fn make_symbol(name: &str, qname: &str, kind: SymbolKind, scope: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope.map(|s| s.to_string()),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line,
        module: None,
        chain: None,
        byte_offset: if line > 0 { 1 } else { 0 },
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        col: 0,
    }
}

fn make_import_ref(source_idx: usize, target: &str, module: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: if line > 0 { 1 } else { 0 },
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        col: 0,
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
        flow: crate::types::FlowMeta::default(),
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

/// Parse an MDX `source` exactly as the indexer does: run the host extractor
/// for JSX `Calls` refs, then sub-extract each emitted `ScriptBlock` region as
/// TypeScript and merge the import refs into the host file's ref vector. The
/// result mirrors `parse_file`'s post-splice `ParsedFile`, so the import
/// bindings the ESM region carries land in `pf.refs` with their `module` field
/// intact — the data `build_file_context` reads to populate
/// `FileContext.imports`.
fn parse_mdx(path: &str, source: &str) -> ParsedFile {
    let host = super::extract::extract(source, path);
    let mut symbols = host.symbols;
    let mut refs = host.refs;
    for region in super::embedded::detect_regions(source) {
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
    make_file(path, "mdx", symbols, refs)
}

#[test]
fn relative_astro_import_binds_jsx_call_through_full_pipeline() {
    // The dominant Track-G shape: an MDX page imports a project-defined
    // component via a relative `.astro` specifier and uses it as a JSX tag.
    // The import sits in the ESM body region (sub-extracted as TypeScript);
    // the `<Card />` tag is a host `Calls` ref. The file-import / component-
    // import rungs must bind the tag to the internal `Card` symbol — proving
    // the ESM region reached `FileContext.imports`.
    let card = make_file(
        "src/components/Card.astro",
        "astro",
        vec![make_symbol("Card", "Card", SymbolKind::Class, None)],
        vec![],
    );
    let page = parse_mdx(
        "src/content/docs/page.mdx",
        "import { Card } from '../../components/Card.astro';\n\n# Title\n\n<Card title=\"Hi\" />\n",
    );

    let (index, id_map) = build_env(&[&page, &card]);
    let file_ctx = MdxHooks.build_file_context(&page, None).unwrap();
    assert!(
        file_ctx
            .imports
            .iter()
            .any(|e| e.imported_name == "Card"),
        "ESM body import must populate FileContext.imports; got {:?}",
        file_ctx.imports
    );

    let call = page
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Card")
        .expect("host extractor must emit a Calls ref for <Card />");
    let ref_ctx = RefContext {
        extracted_ref: call,
        source_symbol: &page.symbols[0],
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
fn bare_specifier_import_binds_when_package_indexed_internal() {
    // Components imported from a bare specifier (`@scope/pkg/components`) bind
    // only when that package is materialized in-index (workspace shape): the
    // module's slash-form must appear as a segment-bounded run inside the
    // defining file's path. Here the package source lives under a matching
    // path, so `resolve_via_component_import` binds the tag. A bare import to a
    // non-indexed package would instead fall through to the external
    // classifier — out of scope for this rung.
    let comp = make_file(
        "node_modules/@scope/pkg/components/Card.ts",
        "typescript",
        vec![make_symbol("Card", "Card", SymbolKind::Class, None)],
        vec![],
    );
    let page = parse_mdx(
        "src/page.mdx",
        "import { Card } from '@scope/pkg/components';\n\n<Card />\n",
    );

    let (index, id_map) = build_env(&[&page, &comp]);
    let file_ctx = MdxHooks.build_file_context(&page, None).unwrap();
    let call = page
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Card")
        .expect("host extractor must emit a Calls ref for <Card />");
    let ref_ctx = RefContext {
        extracted_ref: call,
        source_symbol: &page.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = run_resolve(&file_ctx, &ref_ctx, &index)
        .expect("bare-specifier <Card /> must bind the in-index package component");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "node_modules/@scope/pkg/components/Card.ts".to_string(),
                "Card".to_string()
            ))
            .unwrap()
    );
}

#[test]
fn multiple_imports_one_block_bind_nested_tabitem_usage() {
    // One ESM region declaring several components, used as nested JSX tags.
    // Each tag must bind its own import — confirms the merged region carries
    // every binding, not just the first.
    let tabs = make_file(
        "src/ui/Tabs.astro",
        "astro",
        vec![make_symbol("Tabs", "Tabs", SymbolKind::Class, None)],
        vec![],
    );
    let tab_item = make_file(
        "src/ui/TabItem.astro",
        "astro",
        vec![make_symbol("TabItem", "TabItem", SymbolKind::Class, None)],
        vec![],
    );
    let page = parse_mdx(
        "src/page.mdx",
        "import { Tabs } from './ui/Tabs.astro';\nimport { TabItem } from './ui/TabItem.astro';\n\n<Tabs>\n  <TabItem label=\"One\">a</TabItem>\n</Tabs>\n",
    );

    let (index, id_map) = build_env(&[&page, &tabs, &tab_item]);
    let file_ctx = MdxHooks.build_file_context(&page, None).unwrap();

    let resolve_tag = |name: &str| {
        let call = page
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == name)
            .unwrap_or_else(|| panic!("expected a Calls ref for <{name}>"));
        let ref_ctx = RefContext {
            extracted_ref: call,
            source_symbol: &page.symbols[0],
            scope_chain: build_scope_chain(None),
            file_package_id: None,
        };
        run_resolve(&file_ctx, &ref_ctx, &index)
            .unwrap_or_else(|| panic!("<{name}> must bind its imported component"))
    };

    assert_eq!(
        resolve_tag("Tabs").target_symbol_id,
        *id_map
            .get(&("src/ui/Tabs.astro".to_string(), "Tabs".to_string()))
            .unwrap()
    );
    assert_eq!(
        resolve_tag("TabItem").target_symbol_id,
        *id_map
            .get(&("src/ui/TabItem.astro".to_string(), "TabItem".to_string()))
            .unwrap()
    );
}

#[test]
fn markdown_content_still_extracts_with_import_block_present() {
    // Regression: adding an ESM import block must not disturb the host scan of
    // ordinary Markdown — headings stay Field symbols, fenced code stays inert,
    // and the relative-link Imports ref is still emitted.
    let page = parse_mdx(
        "docs/guide.mdx",
        "import { Card } from './Card.astro';\n\n# Heading One\n\n## Heading Two\n\nSee [more](./info.md).\n\n```ts\nconst x: number = 1;\n```\n",
    );

    let headings: Vec<&str> = page
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(headings, vec!["Heading One", "Heading Two"]);

    assert!(
        page.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Imports && r.target_name == "info"),
        "relative markdown link must still emit an Imports ref"
    );
}

#[test]
fn jsx_calls_dispatched_to_ts_resolver_for_same_file_export() {
    // The MDX host emits the JSX `Calls` ref against the file's host
    // class symbol, while the synthetic TS `ScriptBlock` region's
    // `export function Button() {}` lands in the same `pf.symbols`
    // (post-splice). The TS resolver's same-file lookup binds them.
    // This proves the dispatcher routes JSX Calls into the TS path —
    // the Markdown resolver alone returns None for non-Imports refs
    // and the test would fail without the new dispatcher.
    let mdx = make_file(
        "docs/page.mdx",
        "mdx",
        vec![
            make_symbol("page", "page", SymbolKind::Class, None),
            make_symbol("Button", "Button", SymbolKind::Function, None),
        ],
        vec![make_ref(0, "Button", EdgeKind::Calls, 5)],
    );
    let (index, id_map) = build_env(&[&mdx]);
    let file_ctx = MdxHooks.build_file_context(&mdx, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &mdx.refs[0],
        source_symbol: &mdx.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = run_resolve(&file_ctx, &ref_ctx, &index)
        .expect("JSX Calls ref must bind same-file Button via the engine ladder");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("docs/page.mdx".to_string(), "Button".to_string()))
            .unwrap()
    );
}

#[test]
fn markdown_link_imports_route_through_markdown_resolver() {
    // The MDX host extractor emits link Imports refs identical in shape to
    // Markdown's; `import_resolution` (`resolve_via_import_path`) must bind
    // those by file path, not the TypeScript-shaped strategies.
    let target = make_file(
        "docs/api/overview.md",
        "markdown",
        vec![make_symbol("overview", "overview", SymbolKind::Class, None)],
        vec![],
    );
    let mdx = make_file(
        "docs/page.mdx",
        "mdx",
        vec![make_symbol("page", "page", SymbolKind::Class, None)],
        vec![make_ref(0, "api/overview", EdgeKind::Imports, 1)],
    );

    let (index, id_map) = build_env(&[&mdx, &target]);
    let file_ctx = MdxHooks.build_file_context(&mdx, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &mdx.refs[0],
        source_symbol: &mdx.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = run_resolve(&file_ctx, &ref_ctx, &index)
        .expect("relative .md link should resolve via import_resolution");
    assert_eq!(res.strategy, "markdown_relative_link");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("docs/api/overview.md".to_string(), "overview".to_string()))
            .unwrap()
    );
}

#[test]
fn jsx_calls_with_no_matching_import_falls_through_to_unresolved() {
    // No TS import binds the JSX target; the dispatcher must NOT pretend
    // to resolve it. The engine will land it in unresolved_refs.
    let mdx = make_file(
        "docs/page.mdx",
        "mdx",
        vec![make_symbol("page", "page", SymbolKind::Class, None)],
        vec![make_ref(0, "Unrelated", EdgeKind::Calls, 5)],
    );
    let (index, _) = build_env(&[&mdx]);
    let file_ctx = MdxHooks.build_file_context(&mdx, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &mdx.refs[0],
        source_symbol: &mdx.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    assert!(
        run_resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "no import → no Tier-1 resolution"
    );
}
