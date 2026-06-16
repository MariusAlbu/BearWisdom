// =============================================================================
// groovy/resolve_tests.rs — unit tests for Groovy hooks (GORM flow emission).
// =============================================================================

use crate::indexer::resolve::legacy::FileContext;

// ---------------------------------------------------------------------------
// Goal 31 — GORM flow emission
// ---------------------------------------------------------------------------

use super::hooks::detect_groovy_gorm_emission;
use crate::types::*;

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: "test".to_string(),
                kind: if i == 0 {
                    SegmentKind::Identifier
                } else {
                    SegmentKind::Property
                },
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
fn test_groovy_gorm_findall_emits_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["User", "findAll"]);
    match detect_groovy_gorm_emission(&chain).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "groovy.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_groovy_gorm_save_emits_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Post", "save"]);
    match detect_groovy_gorm_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_groovy_gorm_delete_emits_delete() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Comment", "delete"]);
    match detect_groovy_gorm_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Delete),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_groovy_gorm_rejects_lowercase_root() {
    let chain = make_chain(&["user", "findAll"]);
    assert!(detect_groovy_gorm_emission(&chain).is_none());
}

#[test]
fn test_groovy_gorm_rejects_unknown_leaf() {
    let chain = make_chain(&["User", "doSomething"]);
    assert!(detect_groovy_gorm_emission(&chain).is_none());
}

#[test]
fn test_groovy_gorm_where_emits_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["User", "where"]);
    match detect_groovy_gorm_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}
