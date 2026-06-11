use super::hooks::{detect_lua_db_emission, detect_lua_lapis_route, detect_lua_resty_http};
use crate::indexer::resolve::engine::{infer_external_from_chain, SymbolIndex};
use crate::types::*;
use std::collections::HashMap;

#[test]
fn test_lua_lapis_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_lua_lapis_route("", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_lua_lapis_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_lapis_route("", "post", &args).is_some());
}

#[test]
fn test_lua_lapis_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_lapis_route("", "middleware", &args).is_none());
}

#[test]
fn test_lua_resty_http_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_lua_resty_http("resty.http", "request_uri", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_lua_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_resty_http("string", "request", &args).is_none());
}

#[test]
fn test_lua_pgmoon_query_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_lua_db_emission("pgmoon", "query").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_lua_db_rejects_non_db_module() {
    assert!(detect_lua_db_emission("io", "query").is_none());
}

// ---------------------------------------------------------------------------
// Colon-call receiver chain: `s:gsub(...)` carries a 2-segment receiver chain
// `[s, gsub]`, so a string-typed receiver roots the leaf to the external
// `string` library (Lua's `string.gsub`). Before the fix the extractor emitted
// a bare `gsub` with chain None, dropping the receiver entirely.
// ---------------------------------------------------------------------------

#[test]
fn test_lua_colon_call_binds_gsub_to_external_string() {
    let src = "function clean(s) return s:gsub('%s+', ' ') end";
    let result = super::extract::extract(src);

    let gsub = result
        .refs
        .iter()
        .find(|r| r.target_name == "gsub" && r.kind == EdgeKind::Calls)
        .expect("expected a Calls ref for the gsub method call");

    // The receiver must survive as a 2-segment chain, not be dropped to None.
    let chain = gsub
        .chain
        .as_ref()
        .expect("s:gsub() must carry a receiver chain, not chain None");
    let names: Vec<&str> = chain.segments.iter().map(|seg| seg.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["s", "gsub"],
        "expected 2-segment receiver chain [s, gsub]"
    );

    // With the receiver typed `string` (post flow-inference), the chain roots to
    // the external `string` library — Lua's `string.gsub` builtin — instead of
    // landing as an unresolved bare call.
    let mut typed = chain.clone();
    typed.segments[0].declared_type = Some("string".to_string());
    let index = SymbolIndex::build(&[], &HashMap::new());
    let ns = infer_external_from_chain(&typed, &[], &index)
        .expect("string receiver should classify the chain as external");
    assert!(
        ns.contains("string"),
        "expected the chain to root to the external `string` library; got {ns:?}"
    );
}
