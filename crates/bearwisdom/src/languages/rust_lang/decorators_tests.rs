// Sibling tests for decorators.rs.
//
// `extract_decorators` no longer emits a TypeRef for the bare attribute
// name (`derive`, `serde`, `tokio`, `route`, ...). The bare names had no
// downstream consumer — the resolver only reads EdgeKind::Imports for
// scope-building, and connectors source-scan the AST directly. The
// previous-shape pairing of `target_name = name, module = first_arg`
// (e.g. `target_name = "route", module = Some("/api/users")`) is gone too.
//
// What IS still emitted:
//   * TypeRefs for each derive trait inside `#[derive(...)]` (Debug, Clone,
//     serde::Serialize, ...). These participate in inheritance/impl edges
//     and feed `synthesize_derive_methods`.

use super::super::extract::extract;
use crate::types::EdgeKind;

fn decorator_refs(source: &str) -> Vec<(String, Option<String>)> {
    extract(source)
        .refs
        .into_iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| (r.target_name, r.module))
        .collect()
}

#[test]
fn derive_emits_inner_trait_names_only() {
    let src = "#[derive(Debug, Clone)]\nstruct Point { x: i32, y: i32 }";
    let dr = decorator_refs(src);
    assert!(dr.iter().any(|(n, _)| n == "Debug"), "refs: {dr:?}");
    assert!(dr.iter().any(|(n, _)| n == "Clone"), "refs: {dr:?}");
    assert!(
        !dr.iter().any(|(n, _)| n == "derive"),
        "bare `derive` attribute name should not be emitted; refs: {dr:?}"
    );
}

#[test]
fn bare_test_attribute_not_emitted() {
    // `#[test]` — bare attribute, no inner names to extract. Nothing
    // should land as a TypeRef from the attribute itself.
    let src = "#[test]\nfn it_works() {}";
    let dr = decorator_refs(src);
    assert!(
        !dr.iter().any(|(n, _)| n == "test"),
        "bare `test` attribute name should not be emitted; refs: {dr:?}"
    );
}

#[test]
fn route_attribute_with_string_arg_not_emitted() {
    // The previous shape paired `target_name = "route"` with
    // `module = Some("/api/users")`. No downstream consumer used that
    // pairing — REST connectors source-scan directly. The whole pair is
    // gone now.
    let src = r#"#[route("/api/users")]
fn users() {}"#;
    let dr = decorator_refs(src);
    assert!(
        !dr.iter().any(|(n, _)| n == "route"),
        "bare `route` attribute should not be emitted; refs: {dr:?}"
    );
}

#[test]
fn multiple_attributes_emit_only_derive_inner_names() {
    let src = "#[derive(Debug)]\n#[serde(rename_all = \"camelCase\")]\nstruct Cfg {}";
    let dr = decorator_refs(src);
    assert!(dr.iter().any(|(n, _)| n == "Debug"), "refs: {dr:?}");
    assert!(
        !dr.iter().any(|(n, _)| n == "derive"),
        "bare `derive` should not be emitted; refs: {dr:?}"
    );
    assert!(
        !dr.iter().any(|(n, _)| n == "serde"),
        "bare `serde` attribute should not be emitted; refs: {dr:?}"
    );
}

#[test]
fn enum_derive_emits_inner_trait_names_only() {
    let src = "#[derive(Debug, PartialEq)]\nenum Status { Active, Inactive }";
    let dr = decorator_refs(src);
    assert!(dr.iter().any(|(n, _)| n == "Debug"), "refs: {dr:?}");
    assert!(dr.iter().any(|(n, _)| n == "PartialEq"), "refs: {dr:?}");
    assert!(
        !dr.iter().any(|(n, _)| n == "derive"),
        "bare `derive` should not be emitted; refs: {dr:?}"
    );
}

#[test]
fn proc_macro_attributes_dont_leak_their_path() {
    // `#[prost(message, optional, tag = "1")]` on a struct field — the
    // `prost` ident must not appear as a TypeRef.
    let src = r#"#[derive(Clone, PartialEq, prost::Message)]
struct X {
    #[prost(message, optional, tag = "1")]
    field: Option<i32>,
}"#;
    let dr = decorator_refs(src);
    assert!(
        !dr.iter().any(|(n, _)| n == "prost"),
        "bare `prost` proc-macro attribute path should not be emitted; refs: {dr:?}"
    );
    // The scoped `prost::Message` derive trait should still produce a TypeRef.
    assert!(
        dr.iter().any(|(n, _)| n == "prost::Message" || n == "Message"),
        "expected derive trait `prost::Message`; refs: {dr:?}"
    );
}
