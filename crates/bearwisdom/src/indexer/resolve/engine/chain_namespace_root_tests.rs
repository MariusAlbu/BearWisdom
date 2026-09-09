use super::*;
use crate::indexer::resolve::engine::testkit;

#[test]
fn absent_lexical_namespace_does_not_intercept_the_existing_value_walker() {
    let lookup = testkit::Lookup::new();
    let reference = testkit::call_ref("api.create");
    let source = testkit::source_symbol("f");
    assert!(anchor(
        &testkit::ref_ctx(&reference, &source, vec![]),
        &testkit::file_ctx(vec![], None),
        &lookup,
        lookup.type_arena().unwrap(),
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE
    )
    .is_none());
}
