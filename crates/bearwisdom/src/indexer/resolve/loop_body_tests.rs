use crate::db::Database;
use crate::indexer::resolve::resolve_and_write;
use crate::indexer::write::write_parsed_files_with_origin;
use crate::types::*;

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn method_symbol(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Method,
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
    }
}

fn class_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
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
    }
}

fn calls_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A `ParsedFile` whose single ref optionally carries an embedded origin
/// language (`ref_origin_languages[0]`). `Some(lang)` makes the ref
/// cross-language embedded; `None` makes it native to `lang`.
fn file_with_embedded_ref(
    path: &str,
    host_lang: &str,
    syms: Vec<ExtractedSymbol>,
    r: ExtractedRef,
    ref_origin: Option<&str>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: host_lang.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs: vec![r],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![ref_origin.map(|s| s.to_string())],
        symbol_from_snippet: vec![],
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn plain_file(path: &str, lang: &str, syms: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: lang.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs: vec![],
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

/// Resolve `parsed` into a fresh in-memory DB and return every edge as
/// `(source_id, target_id, kind, strategy)`.
fn resolve_edges(parsed: &[ParsedFile]) -> Vec<(i64, i64, String, Option<String>)> {
    let mut db = Database::open_in_memory().expect("in-memory db");
    let (_files, symbol_id_map) =
        write_parsed_files_with_origin(&db, parsed, "internal", None).expect("write parsed files");
    resolve_and_write(&mut db, parsed, &symbol_id_map, None).expect("resolve");
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT source_id, target_id, kind, strategy FROM edges")
        .expect("prepare edges select");
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .expect("query edges");
    rows.map(|r| r.expect("edge row")).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn embedded_elixir_ref_binds_to_colocated_heex_view_fn() {
    // A `<%= downloads_link(ep) %>` call in a `.heex` template carries
    // `ref_origin_language="elixir"`, so the engine dispatches it against the
    // embedded Elixir context and the host HEEx hook's co-located *View binding
    // is unreachable on the engine path. The embedded-miss host-hook fallback
    // retries through the HEEx hook, which binds the call to the same-named
    // function in the template's co-located `*View` module.
    let view = plain_file(
        "lib/changelog_web/views/admin/episode_view.ex",
        "elixir",
        vec![method_symbol(
            "downloads_link",
            "ChangelogWeb.Admin.EpisodeView.downloads_link",
        )],
    );
    let template = file_with_embedded_ref(
        "lib/changelog_web/templates/admin/episode/edit.html.heex",
        "heex",
        vec![class_symbol("edit.html")],
        calls_ref("downloads_link"),
        Some("elixir"),
    );

    let edges = resolve_edges(&[view, template]);
    assert_eq!(
        edges.len(),
        1,
        "expected exactly one edge for the embedded template call, got {edges:?}"
    );
    let (_src, _tgt, kind, strategy) = &edges[0];
    assert_eq!(kind, "calls");
    assert_eq!(
        strategy.as_deref(),
        Some("heex_colocated_view_fn"),
        "the embedded template call must bind via the HEEx host hook"
    );
}

#[test]
fn native_elixir_miss_does_not_consult_heex_host_hook() {
    // Negative guard: a non-embedded Elixir ref (host language == Elixir, no
    // embedded origin) that the engine can't resolve must NOT fall through to
    // the HEEx host hook. The fallback fires only for cross-language embedded
    // refs; a native miss stays unresolved even when a same-named function
    // sits in a path the HEEx colocated-view mapping would accept.
    let view = plain_file(
        "lib/changelog_web/views/admin/episode_view.ex",
        "elixir",
        vec![method_symbol(
            "downloads_link",
            "ChangelogWeb.Admin.EpisodeView.downloads_link",
        )],
    );
    // Host language is Elixir and the ref has no embedded origin, so
    // `is_cross_lang_embedded` is false. Path mimics a template location only
    // to prove the HEEx mapping is never consulted for a native ref.
    let caller = file_with_embedded_ref(
        "lib/changelog_web/templates/admin/episode/edit.html.heex",
        "elixir",
        vec![class_symbol("edit.html")],
        calls_ref("downloads_link"),
        None,
    );

    let edges = resolve_edges(&[view, caller]);
    assert!(
        edges
            .iter()
            .all(|(_, _, _, strategy)| strategy.as_deref() != Some("heex_colocated_view_fn")),
        "a native Elixir miss must not bind via the HEEx host hook, got {edges:?}"
    );
}

#[test]
fn construction_initializer_types_local_for_member_chain() {
    // `def analyzer = new C(); analyzer.m()` — the constructor RHS has no
    // return/field type, so the local's type is derived from the Instantiates
    // ref via the engine's expression-type inference. The chain `analyzer.m`
    // then resolves `m` on `C`. Mirrors the groovy-codenarc shape where
    // `def analyzer = new SuppressionAnalyzer(...)` left `isViolationSuppressed`
    // unresolved.
    let class_c = class_symbol("C"); // idx 0 in the file
    let method_m = {
        let mut s = method_symbol("m", "C.m");
        s.scope_path = Some("C".to_string());
        s
    }; // idx 1
    let caller = {
        let mut s = method_symbol("run", "Caller.run");
        s.scope_path = Some("Caller".to_string());
        s
    }; // idx 2
    let local = {
        let mut s = class_symbol("analyzer");
        s.kind = SymbolKind::Variable;
        s
    }; // idx 3

    // Instantiates ref for `new C()`, source = caller (idx 2).
    let inst_ref = ExtractedRef {
        kind: EdgeKind::Instantiates,
        source_symbol_index: 2,
        byte_offset: 20,
        ..calls_ref("C")
    };
    // Chain ref `analyzer.m()`, source = caller (idx 2).
    let chain_ref = ExtractedRef {
        source_symbol_index: 2,
        byte_offset: 40,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "analyzer".to_string(),
                    node_kind: "identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 40,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                },
                ChainSegment {
                    name: "m".to_string(),
                    node_kind: "method_invocation".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 49,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                },
            ],
        }),
        ..calls_ref("m")
    };

    let mut flow = FlowMeta::default();
    // The Instantiates ref (index 0 in refs) initializes the `analyzer` local
    // (symbol idx 3).
    flow.flow_binding_lhs.insert(0, 3);
    flow.ref_byte_offsets = vec![20, 40];

    let file = ParsedFile {
        path: "C.groovy".to_string(),
        language: "groovy".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![class_c, method_m, caller, local],
        refs: vec![inst_ref, chain_ref],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![None, None],
        symbol_from_snippet: vec![],
        flow,
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let edges = resolve_edges(&[file]);
    assert!(
        edges.iter().any(|(_, _, kind, _)| kind == "calls"),
        "expected a `calls` edge for analyzer.m() bound to C.m, got {edges:?}"
    );
}
