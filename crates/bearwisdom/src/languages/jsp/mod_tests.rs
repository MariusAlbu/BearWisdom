use super::JspPlugin;
use crate::languages::java;
use crate::languages::LanguagePlugin;
use crate::types::EdgeKind;

/// A `<%= expr %>` expression scriptlet must not leak the synthetic wrapper's
/// return type as a code reference. The wrapper exists only to give the Java
/// sub-extractor a parseable expression context; its declared return type is
/// scaffolding.
#[test]
fn expression_scriptlet_emits_no_object_type_ref() {
    let regions = JspPlugin.embedded_regions("<%= user.getName() %>", "page.jsp", "jsp");
    let region = regions
        .iter()
        .find(|r| r.language_id == "java")
        .expect("expression scriptlet should emit a Java region");

    let result = java::extract::extract(&region.text);
    let object_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Object")
        .collect();

    assert!(
        object_refs.is_empty(),
        "wrapper leaked a bare Object TypeRef: {object_refs:?}"
    );
}
