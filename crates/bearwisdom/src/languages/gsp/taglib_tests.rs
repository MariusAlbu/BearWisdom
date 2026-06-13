use super::{
    is_grails_markup_tag, is_standard_grails_tag, scan_markup_tags, LOGICAL_MARKUP_TAGS,
    STANDARD_TAGS,
};

#[test]
fn standard_tag_list_is_sorted_for_binary_search() {
    let mut sorted = STANDARD_TAGS.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        STANDARD_TAGS, &sorted[..],
        "STANDARD_TAGS must stay in ascending order for binary_search"
    );
}

#[test]
fn logical_markup_tag_list_is_sorted_for_binary_search() {
    let mut sorted = LOGICAL_MARKUP_TAGS.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        LOGICAL_MARKUP_TAGS, &sorted[..],
        "LOGICAL_MARKUP_TAGS must stay in ascending order for binary_search"
    );
}

#[test]
fn markup_scan_recovers_custom_namespaced_tag() {
    // `<warehouse:message ...>` markup → one tag named by its local part.
    let tags = scan_markup_tags(r#"<title><warehouse:message code="cache.title" default="Cache" /></title>"#);
    assert_eq!(tags.len(), 1, "expected one markup tag: {:?}", tags.iter().map(|t| &t.name).collect::<Vec<_>>());
    assert_eq!(tags[0].name, "message");
}

#[test]
fn markup_scan_recovers_core_and_logical_tags() {
    let tags = scan_markup_tags("<g:if test=\"x\"><g:link controller=\"a\">L</g:link></g:if>");
    let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
    // Open tags only: `g:if`, `g:link`. The closing `</g:link>`/`</g:if>` are skipped.
    assert_eq!(names, vec!["if", "link"], "got {names:?}");
}

#[test]
fn markup_scan_ignores_plain_html_and_closing_tags() {
    // Plain HTML elements have no `namespace:local` colon and emit nothing;
    // the closing form of a namespaced tag is not a fresh invocation.
    let tags = scan_markup_tags("<div class=\"x\"><span>text</span></div>");
    assert!(tags.is_empty(), "plain HTML leaked markup tags: {:?}", tags.iter().map(|t| &t.name).collect::<Vec<_>>());
}

#[test]
fn markup_tag_recognizes_core_render_and_logical_tags() {
    // Standard rendering tags and logical/iteration tags are framework builtins
    // in markup position.
    assert!(is_grails_markup_tag("message"));
    assert!(is_grails_markup_tag("link"));
    assert!(is_grails_markup_tag("if"));
    assert!(is_grails_markup_tag("each"));
    assert!(is_grails_markup_tag("set"));
    assert!(is_grails_markup_tag("unless"));
    // A custom project tag is NOT a framework builtin — it binds to its closure.
    assert!(!is_grails_markup_tag("autoSuggest"));
    assert!(!is_grails_markup_tag("selectLocation"));
}

#[test]
fn logical_tags_are_excluded_from_bare_expression_contract() {
    // `if`/`collect`/`findAll` must not brand a bare `.groovy` expression call —
    // they collide with Groovy keywords / collection methods. Only the markup
    // form (`is_grails_markup_tag`) treats them as tags.
    assert!(!is_standard_grails_tag("if"));
    assert!(!is_standard_grails_tag("collect"));
    assert!(!is_standard_grails_tag("findAll"));
    assert!(!is_standard_grails_tag("grep"));
}

#[test]
fn recognizes_core_link_and_message_tags() {
    assert!(is_standard_grails_tag("message"));
    assert!(is_standard_grails_tag("resource"));
    assert!(is_standard_grails_tag("createLink"));
    assert!(is_standard_grails_tag("hasErrors"));
    assert!(is_standard_grails_tag("fieldValue"));
    assert!(is_standard_grails_tag("formatNumber"));
    assert!(is_standard_grails_tag("render"));
}

#[test]
fn does_not_recognize_codec_or_project_or_collection_methods() {
    // Codec methods are receiver-chained, not bare tags.
    assert!(!is_standard_grails_tag("encodeAsHTML"));
    // Groovy collection methods are not part of the tag contract.
    assert!(!is_standard_grails_tag("collect"));
    assert!(!is_standard_grails_tag("findAll"));
    // Arbitrary project method names.
    assert!(!is_standard_grails_tag("notify"));
    assert!(!is_standard_grails_tag("isCanceled"));
}
