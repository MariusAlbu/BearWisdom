use super::extract::extract;
use crate::types::EdgeKind;

#[test]
fn block_directive_becomes_field_symbol() {
    let src = "{% block content %}body{% endblock %}";
    let r = extract(src, "page.j2");
    assert!(r.symbols.iter().any(|s| s.name == "content"));
}

#[test]
fn extends_becomes_imports_ref() {
    let src = "{% extends \"base.j2\" %}";
    let r = extract(src, "page.j2");
    assert!(r.refs.iter().any(|r| r.kind == EdgeKind::Imports && r.target_name == "base"));
}

#[test]
fn include_becomes_imports_ref() {
    let src = "{% include \"partials/header.j2\" %}";
    let r = extract(src, "layout.j2");
    assert!(r
        .refs
        .iter()
        .any(|r| r.kind == EdgeKind::Imports && r.target_name == "partials/header"));
}

#[test]
fn dotted_chain_emits_typeref_on_head_only() {
    let src = "{{ user.profile.email }}";
    let r = extract(src, "p.j2");
    let heads: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(heads, vec!["user"]);
}

#[test]
fn pipe_filters_do_not_emit_filter_call_refs() {
    // Regression for the Jinja-as-JS embedding bug. `indent`, `to_nice_yaml`,
    // `regex_replace` MUST NOT appear as refs.
    let src = "{{ thing | list | to_nice_yaml(indent=2) | indent(8) }}";
    let r = extract(src, "tpl.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"thing"));
    assert!(!names.contains(&"indent"));
    assert!(!names.contains(&"to_nice_yaml"));
    assert!(!names.contains(&"list"));
    assert!(!names.contains(&"regex_replace"));
}

#[test]
fn keyword_heads_are_not_emitted() {
    let src = "{{ if cond else fallback }}";
    let r = extract(src, "p.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(!names.contains(&"if"));
    assert!(!names.contains(&"else"));
    // The non-keyword identifiers ARE emitted.
    assert!(names.contains(&"cond"));
    assert!(names.contains(&"fallback"));
}

#[test]
fn comment_block_is_skipped() {
    let src = "{# {{ ignored.identifier }} #}{{ real_ref }}";
    let r = extract(src, "p.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(!names.contains(&"ignored"));
    assert!(names.contains(&"real_ref"));
}

#[test]
fn string_literals_inside_expression_dont_emit_refs() {
    let src = "{{ greeting + \"world.example\" }}";
    let r = extract(src, "p.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"greeting"));
    assert!(!names.contains(&"world"));
    assert!(!names.contains(&"example"));
}

#[test]
fn for_loop_introduces_loop_variable_as_symbol() {
    let src = "{% for vm in vms %}{{ vm.name }}{% endfor %}";
    let r = extract(src, "p.j2");
    assert!(
        r.symbols.iter().any(|s| s.name == "vm"),
        "loop var should become a Variable symbol"
    );
    // Iterable scanned for refs.
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"vms"));
    assert!(names.contains(&"vm"));
}

#[test]
fn for_loop_tuple_binding_introduces_each_name() {
    let src = "{% for k, v in items %}{{ k }}={{ v }}{% endfor %}";
    let r = extract(src, "p.j2");
    let symbol_names: Vec<_> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(symbol_names.contains(&"k"));
    assert!(symbol_names.contains(&"v"));
}

// Regression for the kubespray pattern: `{% for key, value in dict.items() %}`
// — the iterable is a method call on a deep chain. The tuple binding must
// still be recognised even though the iterable spans multiple chained
// identifiers and contains `()`.
#[test]
fn for_loop_tuple_with_method_call_iterable() {
    let src =
        "{% for key, value in some.nested.path.items() %}{{ key }}{{ value }}{% endfor %}";
    let r = extract(src, "p.j2");
    let symbol_names: Vec<_> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(symbol_names.contains(&"key"), "expected `key` symbol, got {:?}", symbol_names);
    assert!(symbol_names.contains(&"value"), "expected `value` symbol, got {:?}", symbol_names);
}

#[test]
fn set_directive_introduces_variable_symbol() {
    let src = "{% set greeting = 'hello' %}{{ greeting }}";
    let r = extract(src, "p.j2");
    assert!(r.symbols.iter().any(|s| s.name == "greeting"));
}

#[test]
fn macro_directive_introduces_function_symbol_and_param_locals() {
    let src = "{% macro button(label, url) %}<a href=\"{{ url }}\">{{ label }}</a>{% endmacro %}";
    let r = extract(src, "p.j2");
    let names: Vec<_> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"button"));
    assert!(names.contains(&"label"));
    assert!(names.contains(&"url"));
}

#[test]
fn if_directive_scans_condition_for_refs() {
    let src = "{% if user.is_admin %}admin{% endif %}";
    let r = extract(src, "p.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"user"));
}

#[test]
fn from_and_import_directives_become_imports_refs() {
    let src = "{% from \"helpers.j2\" import build_url %}";
    let r = extract(src, "p.j2");
    assert!(r
        .refs
        .iter()
        .any(|r| r.kind == EdgeKind::Imports && r.target_name == "helpers"));

    let src2 = "{% import \"macros.j2\" as m %}";
    let r2 = extract(src2, "p.j2");
    assert!(r2
        .refs
        .iter()
        .any(|r| r.kind == EdgeKind::Imports && r.target_name == "macros"));
}
