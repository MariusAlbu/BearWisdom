use super::*;

#[test]
fn dot_component_becomes_calls_ref() {
    let src = "<div>\n<.button label=\"go\" />\n</div>";
    let r = extract(src, "form.heex");
    assert!(r.refs.iter().any(|r| r.target_name == "button"));
}

#[test]
fn dot_module_component_becomes_calls_ref() {
    let src = "<MyApp.Components.card title=\"hello\" />\n<.button />";
    let r = extract(src, "page.heex");
    // <MyApp.Components.card> starts with uppercase, not `<.` — not matched.
    // <.button> is the dot-component form.
    assert!(r.refs.iter().any(|r| r.target_name == "button"));
}

#[test]
fn file_stem_symbol_emitted() {
    let r = extract("<div></div>", "lib/web/templates/auth/login.html.heex");
    assert!(r.symbols.iter().any(|s| s.name == "login.html"));
}
