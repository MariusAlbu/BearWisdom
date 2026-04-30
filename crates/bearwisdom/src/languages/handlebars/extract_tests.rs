use super::*;

#[test]
fn each_block_becomes_symbol() {
    let src = "{{#each items}}<li>{{name}}</li>{{/each}}";
    let r = extract(src, "list.hbs");
    assert!(r.symbols.iter().any(|s| s.name == "each"));
}

#[test]
fn partial_include_becomes_ref() {
    let src = "{{> header}}\n<main>body</main>\n";
    let r = extract(src, "layout.hbs");
    assert!(r.refs.iter().any(|r| r.target_name == "header"));
}

#[test]
fn if_block_becomes_symbol() {
    let src = "{{#if isLoggedIn}}welcome{{/if}}";
    let r = extract(src, "header.hbs");
    assert!(r.symbols.iter().any(|s| s.name == "if"));
}
