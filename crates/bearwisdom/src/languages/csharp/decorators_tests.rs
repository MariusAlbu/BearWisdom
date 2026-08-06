// =============================================================================
// csharp/decorators_tests.rs — attribute ref emission tests
// =============================================================================

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
fn marker_attribute_on_class_carries_canonical_class_name() {
    let src = "[ApiController]\npublic class UsersController {}";
    let dr = decorator_refs(src);
    assert!(
        dr.iter().any(|(n, _)| n == "ApiControllerAttribute"),
        "refs: {dr:?}"
    );
}

#[test]
fn attribute_with_route_arg() {
    let src = "public class C {\n    [HttpGet(\"{id}\")]\n    public User Get(int id) { return null; }\n}";
    let dr = decorator_refs(src);
    let found = dr.iter().find(|(n, _)| n == "HttpGetAttribute");
    assert!(found.is_some(), "refs: {dr:?}");
    assert_eq!(found.unwrap().1, Some("{id}".to_string()));
}

#[test]
fn multiple_attributes() {
    let src = "[ApiController]\n[Route(\"api/[controller]\")]\npublic class C {}";
    let dr = decorator_refs(src);
    assert!(
        dr.iter().any(|(n, _)| n == "ApiControllerAttribute"),
        "refs: {dr:?}"
    );
    assert!(dr.iter().any(|(n, _)| n == "RouteAttribute"), "refs: {dr:?}");
}

#[test]
fn attribute_no_arg() {
    let src = "public class C {\n    [Authorize]\n    public void Act() {}\n}";
    let dr = decorator_refs(src);
    assert!(
        dr.iter().any(|(n, _)| n == "AuthorizeAttribute"),
        "refs: {dr:?}"
    );
}

#[test]
fn suffixed_long_form_not_doubled() {
    let src = "[FactAttribute]\npublic class T {}";
    let dr = decorator_refs(src);
    assert!(dr.iter().any(|(n, _)| n == "FactAttribute"), "refs: {dr:?}");
    assert!(
        !dr.iter().any(|(n, _)| n == "FactAttributeAttribute"),
        "refs: {dr:?}"
    );
}

#[test]
fn qualified_attribute_suffixes_final_segment() {
    let src = "[Xunit.Fact]\npublic class T {}";
    let dr = decorator_refs(src);
    assert!(
        dr.iter().any(|(n, _)| n == "Xunit.FactAttribute"),
        "refs: {dr:?}"
    );
}
