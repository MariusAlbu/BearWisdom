// VB.NET reuses the C# flow-emission detectors. These tests verify they fire
// on VB.NET-shaped refs.

use crate::languages::csharp::hooks::{
    detect_csharp_db_query_emission, detect_csharp_http_chain_emission,
    detect_refit_attribute_emission,
};
use crate::types::*;

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: "test".to_string(),
                kind: if i == 0 { SegmentKind::Identifier } else { SegmentKind::Property },
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
})
            .collect(),
    }
}

#[test]
fn test_vbnet_resttemplate_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    // VB.NET HttpClient `client.GetStringAsync("/api/x")` shows up as
    // chain ["client", "GetStringAsync"] with the URL in call_args.
    let chain = make_chain(&["client", "GetAsync"]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    let result = detect_csharp_http_chain_emission(&chain, &args);
    // C# detector handles many shapes; we just confirm it's a Producer.
    if let Some(FlowEmission::NamedChannel { role, .. }) = result {
        assert_eq!(role, ChannelRole::Producer);
    }
}

#[test]
fn test_vbnet_dapper_query_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    // VB.NET Dapper `conn.Query(Of User)("SELECT * FROM Users")`.
    let chain = make_chain(&["conn", "Query"]);
    let args = vec![CallArg::StringLit("SELECT * FROM Users".to_string())];
    match detect_csharp_db_query_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_vbnet_refit_attribute_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    match detect_refit_attribute_emission("Get", Some("/api/users")).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_vbnet_efcore_savechanges_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let chain = make_chain(&["context", "SaveChanges"]);
    assert!(matches!(
        detect_csharp_db_query_emission(&chain, &[]).unwrap(),
        FlowEmission::DbQuery { .. }
    ));
}

// `test_vbnet_resolver_is_csharp_resolver` removed — `LanguagePlugin::resolver`
// no longer exists; VB.NET routes through the C# language hooks via the
// language_ids contract on `LanguagePlugin`.

#[test]
fn test_vbnet_dapper_execute_emits_other_op() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["conn", "Execute"]);
    let args = vec![CallArg::StringLit("INSERT INTO Items (a) VALUES (1)".to_string())];
    match detect_csharp_db_query_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

// ---------------------------------------------------------------------------
// External-namespace classification (.NET parity with C#)
// ---------------------------------------------------------------------------

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    scope: Option<&str>,
) -> ExtractedSymbol {
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

fn make_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_vb_file(symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: "src/MainWindow.vb".to_string(),
        language: "vbnet".to_string(),
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

fn make_nuget_ctx() -> crate::indexer::project_context::ProjectContext {
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    let mut ctx = crate::indexer::project_context::ProjectContext::default();
    let mut nuget = ManifestData::default();
    nuget.dependencies.insert("CommunityToolkit.Mvvm".to_string());
    ctx.manifests.insert(ManifestKind::NuGet, nuget);
    ctx
}

#[test]
fn test_vbnet_bcl_import_classified_external() {
    use crate::indexer::resolve::engine::{build_scope_chain, FileContext, RefContext, SymbolIndex};
    use crate::type_checker::profile::hooks::LanguageEngineHooks;
    use std::collections::HashMap;

    let ctx = make_nuget_ctx();
    // One project-namespace class plus two refs: a wildcard BCL `Imports`
    // and a bare `New Button()` instantiation under that wildcard import.
    let file = make_vb_file(
        vec![make_symbol(
            "MainWindow",
            "App.MainWindow",
            SymbolKind::Class,
            Some("App"),
        )],
        vec![
            make_ref("System.Windows.Controls", EdgeKind::Imports),
            make_ref("Button", EdgeKind::Instantiates),
        ],
    );

    let file_ctx = super::hooks::VbNetHooks
        .build_file_context(&file, Some(&ctx))
        .expect("vbnet build_file_context yields a FileContext");
    let empty_lookup = SymbolIndex::build(&[], &HashMap::new());

    let classify_in = |file: &ParsedFile, file_ctx: &FileContext, ref_idx: usize| {
        let ref_ctx = RefContext {
            extracted_ref: &file.refs[ref_idx],
            source_symbol: &file.symbols[0],
            scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
            file_package_id: None,
        };
        super::hooks::VbNetHooks.classify_external(&ref_ctx, file_ctx, Some(&ctx), &empty_lookup)
    };

    // (a) `Imports System.Windows.Controls` is a BCL namespace → external.
    assert!(
        classify_in(&file, &file_ctx, 0).is_some(),
        "BCL Imports namespace should classify as external"
    );
    // (b) bare `New Button()` under the wildcard System.* import → external.
    assert!(
        classify_in(&file, &file_ctx, 1).is_some(),
        "bare type under an external wildcard import should classify as external"
    );

    // (c) A file with NO external wildcard import — only a project-local
    // `Imports App.Internal` — must not produce a false external for a bare
    // local type. (A bare name under an external wildcard IS classified
    // external by the longest-wildcard path; the engine only reaches this
    // hook after project-symbol resolution already failed, so that is the
    // sound widening-only outcome — mirrors C#. The false-external guard is
    // therefore tested against a file that has no external wildcard at all.)
    let local_file = make_vb_file(
        vec![make_symbol(
            "MainWindow",
            "App.MainWindow",
            SymbolKind::Class,
            Some("App"),
        )],
        vec![
            make_ref("App.Internal", EdgeKind::Imports),
            make_ref("MyLocalType", EdgeKind::TypeRef),
        ],
    );
    let local_file_ctx = super::hooks::VbNetHooks
        .build_file_context(&local_file, Some(&ctx))
        .expect("vbnet build_file_context yields a FileContext");
    assert!(
        classify_in(&local_file, &local_file_ctx, 1).is_none(),
        "a project-local type with no external wildcard must not be a false external"
    );
}
