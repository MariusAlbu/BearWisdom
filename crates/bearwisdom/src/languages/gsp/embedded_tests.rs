// Verifies the GSP `${expr}` → embedded-groovy seam: calls from embedded
// expressions are extracted, and `$`/empty Calls targets do not leak.

use super::GspPlugin;
use crate::languages::groovy;
use crate::languages::LanguagePlugin;
use crate::types::EdgeKind;

/// Extract the groovy embedded region(s) the GSP host emits for a fixture,
/// parse each through the groovy extractor, and return every Calls ref target.
fn embedded_call_targets(gsp: &str) -> Vec<String> {
    let regions = GspPlugin.embedded_regions(gsp, "view.gsp", "gsp");
    regions
        .into_iter()
        .filter(|r| r.language_id == "groovy")
        .flat_map(|r| {
            groovy::extract::extract(&r.text)
                .refs
                .into_iter()
                .filter(|rf| rf.kind == EdgeKind::Calls)
                .map(|rf| rf.target_name)
        })
        .collect()
}

#[test]
fn gsp_expression_call_preserved_without_dollar_marker() {
    // A plain GSP expression: the host strips `${` and `}`, wraps the inner
    // text as `def x = (fieldValue(bean: it))`, and the groovy extractor
    // produces a `fieldValue` Calls ref. No `$` node arises here.
    let targets = embedded_call_targets("<div>${fieldValue(bean: it)}</div>");
    assert!(
        targets.iter().any(|t| t == "fieldValue"),
        "GSP expression call `fieldValue` was dropped: {targets:?}"
    );
    assert!(
        !targets.iter().any(|t| t == "$" || t.is_empty()),
        "GSP interpolation leaked a `$`/empty Calls target: {targets:?}"
    );
}

#[test]
fn gsp_expression_with_dollar_call_suppresses_dollar() {
    // A GSP expression whose inner content is itself a `$()` navigator call.
    // The host strips the outer `${ }` and wraps as `def x = ($("sel"))`.
    // The groovy extractor sees `$("sel")` — a `method_invocation` with name
    // `$` — which must be suppressed without emitting an unresolvable ref.
    let targets = embedded_call_targets(r#"<div>${$("sel")}</div>"#);
    assert!(
        !targets.iter().any(|t| t == "$" || t.is_empty()),
        "GSP `$()` inside expression leaked a `$`/empty Calls target: {targets:?}"
    );
}
