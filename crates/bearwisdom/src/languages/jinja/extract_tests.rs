use super::extract::extract;
use super::expr::scan_expression;
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

#[test]
fn raw_block_content_is_not_scanned() {
    // Go template syntax inside raw blocks must not produce refs — `.Sender`,
    // `end`, `Caption`, etc. are not Jinja2 identifiers.
    let src = concat!(
        "{{ real_var }}\n",
        "{% raw %}\n",
        "{{ .Sender.DisambiguatedName }}: {{ .Message }}{{ if .Caption }}: {{ .Caption }}{{ end }}\n",
        "{% endraw %}\n",
        "{{ also_real }}",
    );
    let r = extract(src, "tpl.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"real_var"), "before-raw ref should be emitted");
    assert!(names.contains(&"also_real"), "after-raw ref should be emitted");
    assert!(
        !names.contains(&"Sender"),
        "Go template field inside raw block must be suppressed"
    );
    assert!(
        !names.contains(&"Caption"),
        "Go template field inside raw block must be suppressed"
    );
    assert!(
        !names.contains(&"end"),
        "Go `end` keyword inside raw block must be suppressed"
    );
}

#[test]
fn raw_block_with_trim_markers_is_skipped() {
    let src = "{%- raw -%}{{ .GoProp }}{%- endraw -%}{{ jinja_var }}";
    let r = extract(src, "tpl.j2");
    let names: Vec<_> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(!names.contains(&"GoProp"));
    assert!(names.contains(&"jinja_var"));
}

#[test]
fn nested_pipe_filter_in_parens_is_suppressed() {
    // `([ x ] | flatten)` — `flatten` follows `|` inside parens and must not
    // be emitted as a TypeRef.
    let mut refs = Vec::new();
    scan_expression("([ mirror_list ] | flatten) | join(',')", 0, 0, &mut refs);
    let names: Vec<_> = refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"mirror_list"));
    assert!(!names.contains(&"flatten"), "filter after nested `|` must be suppressed");
    assert!(!names.contains(&"join"), "filter after top-level `|` must be suppressed");
}

#[test]
fn paren_filter_chain_like_matrix_synapse() {
    // Pattern from matrix-synapse: `(x | int | to_json) if cond else ''`
    let mut refs = Vec::new();
    scan_expression(
        "((cache_size | int | to_json) if cache_size else '')",
        0,
        0,
        &mut refs,
    );
    let names: Vec<_> = refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"cache_size"));
    assert!(!names.contains(&"int"), "`int` is a filter name here, not a variable");
    assert!(!names.contains(&"to_json"), "`to_json` is a filter name here, not a variable");
}

#[test]
fn subscript_chain_only_emits_head() {
    // `vm.networkProfile.networkInterfaces[0].expanded.ipAddress` — only
    // `vm` should be emitted; `expanded` and `ipAddress` are chain members.
    let mut refs = Vec::new();
    scan_expression(
        "vm.networkProfile.networkInterfaces[0].expanded.ipAddress",
        0,
        0,
        &mut refs,
    );
    let names: Vec<_> = refs.iter().map(|r| r.target_name.as_str()).collect();
    assert_eq!(names, vec!["vm"], "only the chain head should be emitted");
}

#[test]
fn multiple_subscript_levels_emit_head_only() {
    let mut refs = Vec::new();
    scan_expression("data[key][0].value.sub", 0, 0, &mut refs);
    let names: Vec<_> = refs.iter().map(|r| r.target_name.as_str()).collect();
    // `data` is the head; `key` is the first subscript arg (separate expr),
    // `value` and `sub` are chain continuations after the second `]`.
    assert!(names.contains(&"data"));
    assert!(!names.contains(&"value"), "chain member after subscript must not be emitted");
    assert!(!names.contains(&"sub"), "chain member after subscript must not be emitted");
}
