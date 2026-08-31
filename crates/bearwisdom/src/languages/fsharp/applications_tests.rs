// Chain emission for F# dotted refs: every dotted identifier splits into a
// leaf target plus a full MemberChain (root included) so the generic
// ROOT→MEMBER walk can anchor on the root. Single-segment refs keep
// `chain: None`; the keyword / capitalization gates are unchanged.

use super::super::extract::extract;
use crate::types::{EdgeKind, ExtractedRef, SegmentKind};

fn calls(refs: &[ExtractedRef]) -> Vec<&ExtractedRef> {
    refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect()
}

#[test]
fn dotted_application_callee_splits_into_leaf_and_chain() {
    let r = extract("module M\nlet f xs = List.map xs\nlet g y = y\n");
    let map_ref = calls(&r.refs)
        .into_iter()
        .find(|rf| rf.target_name == "map")
        .expect("List.map application must emit a `map` Calls ref");
    let chain = map_ref.chain.as_ref().expect("dotted ref must carry a chain");
    assert_eq!(chain.segments.len(), 2, "chain carries root + leaf");
    assert_eq!(chain.segments[0].name, "List");
    assert_eq!(
        chain.segments[0].kind,
        SegmentKind::NamespaceAccess,
        "capitalized root is a namespace/module access"
    );
    assert_eq!(chain.segments[1].name, "map");
    assert_eq!(chain.segments[1].kind, SegmentKind::Property);
    assert!(
        !calls(&r.refs).iter().any(|rf| rf.target_name == "List.map"),
        "no dotted target string survives without a chain"
    );
}

#[test]
fn dot_member_application_carries_receiver_root() {
    let r = extract("module M\nlet f obj = obj.Method 1\nlet g y = y\n");
    let m = calls(&r.refs)
        .into_iter()
        .find(|rf| rf.target_name == "Method")
        .expect("obj.Method application must emit a `Method` Calls ref");
    let chain = m.chain.as_ref().expect("receiver spine must survive as chain");
    assert_eq!(chain.segments[0].name, "obj", "chain roots on the receiver");
    assert_eq!(
        chain.segments[0].kind,
        SegmentKind::Identifier,
        "lowercase value root is an identifier, not a namespace"
    );
    assert_eq!(chain.segments.last().unwrap().name, "Method");
}

#[test]
fn capitalized_dotted_value_ref_is_chained() {
    let r = extract("module M\nlet c = CultureInfo.InvariantCulture\nlet y = 0\n");
    let v = calls(&r.refs)
        .into_iter()
        .find(|rf| rf.target_name == "InvariantCulture")
        .expect("CultureInfo.InvariantCulture must emit an `InvariantCulture` ref");
    let chain = v.chain.as_ref().expect("dotted value ref must carry a chain");
    assert_eq!(chain.segments[0].name, "CultureInfo");
    assert_eq!(chain.segments[0].kind, SegmentKind::NamespaceAccess);
}

#[test]
fn bare_application_keeps_chain_none() {
    let r = extract("module M\nlet g x = f x\nlet h y = y\n");
    let bare = calls(&r.refs)
        .into_iter()
        .find(|rf| rf.target_name == "f")
        .expect("bare `f x` must emit an `f` Calls ref");
    assert!(bare.chain.is_none(), "single-segment ref keeps chain: None");
}

#[test]
fn keyword_callee_stays_gated() {
    let r = extract("module M\nlet g x = failwith x\nlet h y = y\n");
    assert!(
        !calls(&r.refs).iter().any(|rf| rf.target_name == "failwith"),
        "keyword callees must not emit Calls refs"
    );
}

#[test]
fn lowercase_value_position_stays_gated() {
    let r = extract("module M\nlet n opt = opt.Value\nlet y = 0\n");
    assert!(
        !r.refs
            .iter()
            .any(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains("Value")),
        "lowercase-rooted value-position reads stay behind the capitalization gate"
    );
}
