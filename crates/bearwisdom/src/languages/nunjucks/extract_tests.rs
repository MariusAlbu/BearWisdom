use super::extract;

#[test]
fn block_directive_becomes_symbol() {
    let src = "{% block content %}hi{% endblock %}";
    let r = extract(src, "page.njk");
    assert!(r.symbols.iter().any(|s| s.name == "content"));
}

#[test]
fn extends_becomes_imports_ref() {
    let src = "{% extends \"base.njk\" %}\n{% block body %}x{% endblock %}\n";
    let r = extract(src, "page.njk");
    assert!(r.refs.iter().any(|r| r.target_name == "base"));
}

#[test]
fn include_becomes_imports_ref() {
    let src = "{% include \"partials/header.njk\" %}";
    let r = extract(src, "layout.njk");
    assert!(r.refs.iter().any(|r| r.target_name == "partials/header"));
}
