use super::*;
use crate::indexer::resolve::engine::testkit;

#[test]
fn unmigrated_call_does_not_claim_a_qualified_anchor() {
    let lookup = testkit::Lookup::new();
    let reference = testkit::call_ref("api.create");
    let source = testkit::source_symbol("f");
    assert!(anchor(
        &testkit::ref_ctx(&reference, &source, vec![]),
        &lookup,
        lookup.type_arena().unwrap(),
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE
    )
    .is_none());
}
