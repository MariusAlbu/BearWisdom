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

#[test]
fn quoted_partial_include_strips_quotes() {
    let src = "{{> \"post-card\"}}";
    let r = extract(src, "feed.hbs");
    assert!(
        r.refs.iter().any(|r| r.target_name == "post-card"),
        "quoted partial name should strip quotes; got: {:?}",
        r.refs.iter().map(|r| r.target_name.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn partial_block_marker_not_emitted_as_partial_import() {
    let src = "{{> @partial-block}}";
    let r = extract(src, "layout.hbs");
    assert!(
        !r.refs.iter().any(|r| r.target_name == "@partial-block"),
        "@partial-block is a placeholder, not a partial path; got: {:?}",
        r.refs.iter().map(|r| r.target_name.as_str()).collect::<Vec<_>>()
    );
}
