use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, LanguageResolver};
use crate::types::*;

fn make_ref_calls(target: &str, args: Vec<CallArg>) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: args,
    }
}

fn make_sym() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "main".to_string(),
        qualified_name: "main".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn make_file_ctx() -> FileContext {
    FileContext {
        file_path: "main.zig".to_string(),
        language: "zig".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    }
}

#[test]
fn test_zig_http_fetch_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let r = make_ref_calls("fetch", vec![CallArg::StringLit("https://api.example.com/x".to_string())]);
    let sym = make_sym();
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let fc = make_file_ctx();
    let emissions = ZigResolver.detect_flow_emission(&fc, &rc);
    assert!(matches!(emissions.first(), Some(FlowEmission::NamedChannel { role: ChannelRole::Producer, .. })));
}

#[test]
fn test_zig_http_send_with_path_emits() {
    let r = make_ref_calls("send", vec![CallArg::StringLit("/api/users".to_string())]);
    let sym = make_sym();
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let fc = make_file_ctx();
    assert!(!ZigResolver.detect_flow_emission(&fc, &rc).is_empty());
}

#[test]
fn test_zig_no_emit_for_non_url_arg() {
    let r = make_ref_calls("fetch", vec![CallArg::StringLit("not-a-url".to_string())]);
    let sym = make_sym();
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let fc = make_file_ctx();
    assert!(ZigResolver.detect_flow_emission(&fc, &rc).is_empty());
}
