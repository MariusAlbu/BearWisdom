use super::*;
use crate::indexer::resolve::engine::cause::{Cause, CauseKind};
use crate::indexer::resolve::engine::chain::bind_member_access;
use crate::indexer::resolve::engine::testkit::{
    call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{ChainSegment, MemberChain, SegmentKind};

/// A language whose declarations inherit the root without writing it.
static ROOTED: LanguageProfile = LanguageProfile {
    implicit_root_types: &["Object"],
    ..DEFAULT_PROFILE
};

/// The external file an indexed root declaration lives in.
const ROOT_FILE: &str = "ext:dotnet-type:CoreLib.dll!!System!!Object";

fn seg(name: &str, is_call: bool, kind: SegmentKind) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

/// A receiver typed `JobResult` — a class declared with NO base list.
fn baseless_receiver() -> Lookup {
    Lookup::new()
        .with(sym(1, "result", "M.result", "parameter", "src/M.cs"))
        .with_field_type("M.result", "JobResult")
        .with(sym(10, "JobResult", "JobResult", "class", "src/JobResult.cs"))
}

/// The indexed root declaration, carrying `ToString`.
fn with_root_tostring(lookup: Lookup) -> Lookup {
    lookup
        .with(sym(90, "Object", "System.Object", "class", ROOT_FILE))
        .with_member(
            "System.Object",
            sym(91, "ToString", "System.Object.ToString", "method", ROOT_FILE),
        )
}

/// Drive `result.ToString()` through the chain walk under `profile`.
fn resolve(lookup: &Lookup, profile: &LanguageProfile) -> Result<i64, Option<Cause>> {
    let segs = vec![
        seg("result", false, SegmentKind::Identifier),
        seg("ToString", true, SegmentKind::Property),
    ];
    let mut r = call_ref("ToString");
    r.chain = Some(MemberChain { segments: segs });
    let mut s = source_symbol("caller");
    s.qualified_name = "M.Run".to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(&rc, &file_ctx(vec![], None), lookup, profile).map(|i| i.target_symbol_id)
}

#[test]
fn baseless_receiver_closes_the_climb_on_the_implicit_root() {
    // `result.ToString()` where `JobResult` declares no base list: the climb
    // exhausts without a hop, so the root's own declaration answers the member.
    let lookup = with_root_tostring(baseless_receiver());
    assert_eq!(resolve(&lookup, &ROOTED).ok(), Some(91));
}

#[test]
fn declared_base_member_wins_over_the_root() {
    // The receiver's own supertype declares the member. The declared climb runs
    // first, so the root — which declares it too — never shadows the override.
    let lookup = with_root_tostring(
        baseless_receiver()
            .with_parent("JobResult", "BaseResult")
            .with(sym(20, "BaseResult", "BaseResult", "class", "src/BaseResult.cs"))
            .with_member(
                "BaseResult",
                sym(21, "ToString", "BaseResult.ToString", "method", "src/BaseResult.cs"),
            ),
    );
    assert_eq!(resolve(&lookup, &ROOTED).ok(), Some(21));
}

#[test]
fn external_root_declaration_wins_over_a_same_named_project_type() {
    // A project class named `Object` shares the root's simple name and declares
    // the member too. The probe must land on the external, qualified row.
    let lookup = with_root_tostring(
        baseless_receiver()
            .with(sym(50, "Object", "Object", "class", "src/Object.cs"))
            .with_member(
                "Object",
                sym(51, "ToString", "Object.ToString", "method", "src/Object.cs"),
            ),
    );
    assert_eq!(resolve(&lookup, &ROOTED).ok(), Some(91));
}

#[test]
fn a_profile_without_root_types_leaves_the_miss() {
    // Same index, a language that declares no implicit root: the walk's miss
    // stands.
    let lookup = with_root_tostring(baseless_receiver());
    let cause = resolve(&lookup, &DEFAULT_PROFILE).expect_err("no root types — the hop must miss");
    assert_eq!(cause.expect("a caused miss").kind, CauseKind::MemberMissing);
}

#[test]
fn an_untyped_receiver_does_not_bind_the_root() {
    // The receiver's type names NO indexed declaration — the walk holds a type
    // string with no bound id. The probe must not fire: binding the root's
    // member on zero type information would hide the upstream capture gap the
    // unresolved ref evidences.
    let lookup = with_root_tostring(
        Lookup::new()
            .with(sym(1, "result", "M.result", "parameter", "src/M.cs"))
            .with_field_type("M.result", "UncapturedType"),
    );
    assert!(
        resolve(&lookup, &ROOTED).is_err(),
        "an untyped receiver keeps its miss"
    );
}

#[test]
fn a_root_without_the_member_stays_a_miss() {
    // The root is indexed but declares no `ToString`: the probe finds nothing
    // and the hop keeps its member-missing cause.
    let lookup = baseless_receiver()
        .with(sym(90, "Object", "System.Object", "class", ROOT_FILE))
        .with_member(
            "System.Object",
            sym(92, "GetHashCode", "System.Object.GetHashCode", "method", ROOT_FILE),
        );
    let cause = resolve(&lookup, &ROOTED).expect_err("the root lacks the member — a miss");
    let cause = cause.expect("a caused miss");
    assert_eq!(cause.kind, CauseKind::MemberMissing);
    assert_eq!(cause.symbol_id, Some(10), "the receiver's own declaration is blamed");
}
