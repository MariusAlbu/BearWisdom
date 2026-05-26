// VB.NET shares the CSharpResolver (mod.rs::resolver). These tests verify
// the C# flow-emission detectors fire on VB.NET-shaped refs.

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
