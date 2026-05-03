use super::*;

#[test]
fn dash_code_line_becomes_js_region() {
    let src = "- const x = getUser()\nh1= x.name\n";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("getUser")));
}

#[test]
fn eq_expression_becomes_js_region() {
    let src = "h1= userName\n";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("userName")));
}

#[test]
fn hash_interpolation_becomes_js_region() {
    let src = "p Hello #{userName}!\n";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("userName")));
}

#[test]
fn script_block_captures_body() {
    let src = "script.\n  console.log(hello)\n  const x = 1\np text\n";
    let regions = detect_regions(src);
    let script = regions
        .iter()
        .find(|r| r.origin == EmbeddedOrigin::ScriptBlock)
        .unwrap();
    assert!(script.text.contains("console.log"));
    assert!(script.text.contains("const x"));
}

#[test]
fn hash_interpolation_declares_i18n_helpers_as_locals() {
    let src = "p Hello #{__('Welcome')}!\n";
    let regions = detect_regions(src);
    let r = regions
        .iter()
        .find(|r| r.text.contains("'Welcome'"))
        .expect("interpolation region present");
    assert!(
        r.text.contains("var __ ="),
        "wrapper should bind __ as a local so the JS resolver doesn't \
         leave it unresolved: {}",
        r.text
    );
    assert!(r.text.contains("var __n ="));
}

#[test]
fn eq_expression_declares_i18n_helpers_as_locals() {
    let src = "h1= __('Title')\n";
    let regions = detect_regions(src);
    let r = regions
        .iter()
        .find(|r| r.text.contains("'Title'"))
        .expect("expression region present");
    assert!(r.text.contains("var __ ="));
}

#[test]
fn dash_code_block_declares_i18n_helpers_as_locals() {
    let src = "- const greeting = __('Hi')\n";
    let regions = detect_regions(src);
    let r = regions
        .iter()
        .find(|r| r.text.contains("greeting"))
        .expect("code region present");
    assert!(r.text.contains("var __ ="));
}
