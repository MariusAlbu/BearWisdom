// groovy/hooks_tests.rs — GSP-host standard-taglib decline.

use super::hooks::gsp_standard_tag;
use crate::indexer::resolve::engine::{FileContext, RefContext};
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};

fn host_ctx(file_path: &str) -> FileContext {
    FileContext {
        file_path: file_path.to_string(),
        language: "groovy".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    }
}

fn source_symbol() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "_view".to_string(),
        qualified_name: "_view".to_string(),
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

fn call_ref(target: &str, chain: Option<MemberChain>) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn one_segment_chain(name: &str) -> MemberChain {
    MemberChain {
        segments: vec![ChainSegment {
            name: name.to_string(),
            node_kind: "test".to_string(),
            kind: SegmentKind::Property,
            declared_type: None,
            type_args: vec![],
            optional_chaining: false,
            byte_offset: 0,
            declared_type_id: None,
            is_call: true,
            call_args: Vec::new(),
            type_arg_ids: Vec::new(),
        }],
    }
}

fn classify(file_path: &str, r: &ExtractedRef) -> Option<String> {
    let sym = source_symbol();
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };
    gsp_standard_tag(&ref_ctx, &host_ctx(file_path))
}

#[test]
fn bare_standard_tag_in_gsp_host_is_branded_taglib() {
    // `${message(code:'x')}` in a `.gsp` view → standard tag, no in-index target.
    let r = call_ref("message", None);
    assert_eq!(
        classify("grails-app/views/book/_summary.gsp", &r).as_deref(),
        Some("grails-taglib")
    );
    let r2 = call_ref("resource", None);
    assert_eq!(
        classify("grails-app/views/store/show.gsp", &r2).as_deref(),
        Some("grails-taglib")
    );
}

#[test]
fn same_name_in_plain_groovy_host_is_not_declined() {
    // A real project method named `message`/`list` in a `.groovy` file must
    // resolve normally — the decline is gated on a GSP host.
    let r = call_ref("message", None);
    assert!(classify("grails-app/services/NotificationService.groovy", &r).is_none());
}

#[test]
fn chained_call_in_gsp_host_is_not_declined() {
    // `${value.encodeAsHTML()}` — a receiver-chained codec call is not a bare
    // tag, even in a GSP host.
    let r = call_ref("encodeAsHTML", Some(one_segment_chain("encodeAsHTML")));
    assert!(classify("grails-app/views/book/show.gsp", &r).is_none());
    // A standard tag name reached via a receiver is also excluded.
    let r2 = call_ref("render", Some(one_segment_chain("render")));
    assert!(classify("grails-app/views/book/show.gsp", &r2).is_none());
}

#[test]
fn non_standard_bare_call_in_gsp_host_is_not_declined() {
    // A project method called bare in expression scope is left for the normal
    // ladder — it may bind to an indexed symbol.
    let r = call_ref("notify", None);
    assert!(classify("grails-app/views/book/show.gsp", &r).is_none());
}
