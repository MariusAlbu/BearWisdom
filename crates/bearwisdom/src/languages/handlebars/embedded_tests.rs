use super::*;

#[test]
fn expression_becomes_js_region() {
    let src = "<p>{{getUserName()}}</p>";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.language_id == "javascript"
        && r.text.contains("getUserName")));
}

#[test]
fn block_open_close_skipped_as_expressions() {
    let src = "{{#each items}}{{/each}}";
    let regions = detect_regions(src);
    assert!(regions.iter().all(|r| r.language_id != "javascript"));
}

// ---------------------------------------------------------------------------
// handlebars_to_js — helper-call rewriting (the Ghost-template pattern)
// ---------------------------------------------------------------------------

#[test]
fn helper_call_with_string_arg_becomes_js_call() {
    assert_eq!(handlebars_to_js("action \"click\""), "action(\"click\")");
}

#[test]
fn helper_call_with_multiple_args_becomes_js_call() {
    assert_eq!(
        handlebars_to_js("concat \"prefix-\" this.value"),
        "concat(\"prefix-\", this.value)"
    );
}

#[test]
fn helper_call_with_hash_arg_uses_value_only() {
    let out = handlebars_to_js("action \"save\" target=\"model\"");
    assert!(out.starts_with("action(\"save\""), "got: {out}");
    assert!(out.contains("\"model\""), "got: {out}");
}

#[test]
fn sub_expression_unwrapped_to_js_call() {
    assert_eq!(handlebars_to_js("(or a b)"), "or(a, b)");
}

#[test]
fn bare_path_passes_through() {
    assert_eq!(handlebars_to_js("user.name"), "user.name");
    assert_eq!(handlebars_to_js("name"), "name");
}

#[test]
fn parent_context_becomes_js_identifier() {
    assert_eq!(handlebars_to_js(".."), "_HBS_PARENT");
    assert_eq!(handlebars_to_js("../user"), "_HBS_PARENT.user");
}

#[test]
fn data_variable_becomes_js_identifier() {
    assert_eq!(handlebars_to_js("@index"), "_HBS_index");
    assert_eq!(handlebars_to_js("@key"), "_HBS_key");
}

#[test]
fn partial_block_marker_skipped() {
    assert!(handlebars_to_js("@partial-block").is_empty());
}

#[test]
fn hyphenated_identifier_normalized() {
    assert_eq!(handlebars_to_js("post-card"), "post_card");
}

#[test]
fn helper_call_does_not_emit_string_literal_as_calls_ref() {
    let src = "<button {{action \"click\"}}>Go</button>";
    let regions = detect_regions(src);
    let js = regions
        .iter()
        .find(|r| r.language_id == "javascript")
        .expect("expected a JS region");
    assert!(
        js.text.contains("action(\"click\")"),
        "wrapper should emit valid JS call; got: {}",
        js.text
    );
}
