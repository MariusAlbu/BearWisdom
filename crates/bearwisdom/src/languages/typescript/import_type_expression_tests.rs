// =============================================================================
// import_type_expression_tests.rs — Unit tests for parse_import_type_expression.
//
// Covers the syntactic shapes that land as `nested_type_identifier` /
// `member_expression` in the TS extractor and need to be split into a
// (module, type) pair so the standard import-resolution pipeline can
// match them. Real-world cases from the corpus drove these tests:
// `import('node:stream').Readable`, `import('typescript').Diagnostic`,
// `import('svelte').Snippet`.
// =============================================================================

use super::{
    extract_type_ref_from_annotation, import_call_module, import_lookup_type_parts,
    import_type_query_module, parse_import_type_expression,
};
use crate::types::{EdgeKind, ExtractedRef};

/// Find the first named child of `node` with the given kind.
fn child_of<'a>(node: tree_sitter::Node<'a>, kind: &str) -> tree_sitter::Node<'a> {
    (0..node.child_count())
        .filter_map(|i| node.child(i))
        .find(|c| c.kind() == kind)
        .unwrap_or_else(|| panic!("expected child `{kind}` under `{}`", node.kind()))
}

/// Parse `src` as `let x: <type>;`, then call `f` with the type node *inside*
/// the annotation (the child after `:`). The tree is owned for the closure's
/// lifetime so callers borrow the node without it outliving the parse.
fn with_inner_type_node<R>(src: &str, f: impl FnOnce(tree_sitter::Node) -> R) -> R {
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(src, None).unwrap();
    let decl = child_of(tree.root_node(), "lexical_declaration");
    let ann = child_of(child_of(decl, "variable_declarator"), "type_annotation");
    let inner = (0..ann.child_count())
        .filter_map(|i| ann.child(i))
        .find(|c| c.kind() != ":")
        .expect("type node after colon");
    f(inner)
}

/// Run the annotation extractor on the inner type node of `let x: <type>;` and
/// return the emitted (target, module) pairs.
fn refs_for_annotation(src: &str) -> Vec<(String, Option<String>)> {
    with_inner_type_node(src, |inner| {
        let mut refs: Vec<ExtractedRef> = Vec::new();
        extract_type_ref_from_annotation(&inner, src.as_bytes(), 0, &mut refs);
        refs.into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.target_name, r.module))
            .collect()
    })
}

#[test]
fn lookup_type_import_emits_export_key_with_module() {
    // `typeof import('vitest')['expect']` → one ref targeting the index key
    // `expect`, module-tagged `vitest` — the same shape `import { expect }
    // from 'vitest'` produces, so cross-module resolution applies.
    let refs = refs_for_annotation("let e: typeof import('vitest')['expect'];");
    assert_eq!(
        refs,
        vec![("expect".to_string(), Some("vitest".to_string()))]
    );
}

#[test]
fn type_query_import_emits_module_as_target() {
    // `typeof import('vitest')` (whole namespace) → the module name doubles
    // as the target, module-tagged. No raw `import('vitest')` text leaks.
    let refs = refs_for_annotation("let e: typeof import('vitest');");
    assert_eq!(
        refs,
        vec![("vitest".to_string(), Some("vitest".to_string()))]
    );
}

#[test]
fn bare_import_call_emits_module_as_target() {
    // `import('vitest')` as a type annotation (default-type) → module name as
    // target, module-tagged.
    let refs = refs_for_annotation("let e: import('vitest');");
    assert_eq!(
        refs,
        vec![("vitest".to_string(), Some("vitest".to_string()))]
    );
}

#[test]
fn plain_typeof_value_is_not_import_typed() {
    // `typeof someValue` must keep its existing behaviour: a value-space ref
    // with no module. The import-type path must not hijack it.
    let refs = refs_for_annotation("let e: typeof someValue;");
    assert_eq!(refs, vec![("someValue".to_string(), None)]);
}

#[test]
fn non_import_lookup_type_falls_back_to_object_ref() {
    // `Repo['findOne']` is a normal indexed-access lookup, not an import-type.
    // The object head `Repo` is emitted; no module tag.
    let refs = refs_for_annotation("let e: Repo['findOne'];");
    assert_eq!(refs, vec![("Repo".to_string(), None)]);
}

#[test]
fn import_lookup_type_parts_decomposes_module_and_key() {
    let src = "let e: typeof import('vitest')['expect'];";
    let parts = with_inner_type_node(src, |lookup| {
        import_lookup_type_parts(&lookup, src.as_bytes())
    });
    assert_eq!(parts, Some(("vitest".to_string(), "expect".to_string())));
}

#[test]
fn structural_helpers_reject_non_import_nodes() {
    // `typeof someValue`: the `type_query` carries no import call.
    let src = "let e: typeof someValue;";
    with_inner_type_node(src, |tq| {
        let bytes = src.as_bytes();
        assert_eq!(import_type_query_module(&tq, bytes), None);
        assert_eq!(import_call_module(&tq, bytes), None);
        assert_eq!(import_lookup_type_parts(&tq, bytes), None);
    });
}

#[test]
fn single_quoted_module_with_simple_type() {
    let r = parse_import_type_expression("import('node:stream').Readable");
    assert_eq!(r, Some(("node:stream".to_string(), "Readable".to_string())));
}

#[test]
fn double_quoted_module_with_simple_type() {
    let r = parse_import_type_expression("import(\"typescript\").Diagnostic");
    assert_eq!(
        r,
        Some(("typescript".to_string(), "Diagnostic".to_string()))
    );
}

#[test]
fn relative_module_path() {
    let r = parse_import_type_expression("import('../offline').Foo");
    assert_eq!(r, Some(("../offline".to_string(), "Foo".to_string())));
}

#[test]
fn scoped_package() {
    let r = parse_import_type_expression("import('@types/node').ProcessEnv");
    assert_eq!(
        r,
        Some(("@types/node".to_string(), "ProcessEnv".to_string()))
    );
}

#[test]
fn dotted_type_suffix_kept_intact() {
    // `import('foo').Bar.Baz` — the leftmost type plus dotted suffix.
    let r = parse_import_type_expression("import('foo').Bar.Baz");
    assert_eq!(r, Some(("foo".to_string(), "Bar.Baz".to_string())));
}

#[test]
fn whitespace_inside_parens_tolerated() {
    let r = parse_import_type_expression("import( 'foo' ).Bar");
    assert_eq!(r, Some(("foo".to_string(), "Bar".to_string())));
}

#[test]
fn no_type_suffix_emits_module_as_target() {
    // `import('foo')` used as a type annotation — refers to the module's
    // default export. Caller treats the module as the target.
    let r = parse_import_type_expression("import('foo')");
    assert_eq!(r, Some(("foo".to_string(), "foo".to_string())));
}

#[test]
fn missing_import_keyword_returns_none() {
    let r = parse_import_type_expression("foo('node:stream').Readable");
    assert_eq!(r, None);
}

#[test]
fn missing_quotes_returns_none() {
    let r = parse_import_type_expression("import(foo).Bar");
    assert_eq!(r, None);
}

#[test]
fn unclosed_module_string_returns_none() {
    let r = parse_import_type_expression("import('foo");
    assert_eq!(r, None);
}

#[test]
fn empty_type_after_dot_returns_none() {
    let r = parse_import_type_expression("import('foo').");
    assert_eq!(r, None);
}

#[test]
fn missing_dot_with_extra_text_returns_none() {
    // After `)` we expect `.Type` or end-of-input. Anything else is
    // not the import-type shape.
    let r = parse_import_type_expression("import('foo')<T>");
    assert_eq!(r, None);
}

#[test]
fn plain_dotted_name_returns_none() {
    // `chrome.cast.Error` is a regular dotted type, not import-type.
    let r = parse_import_type_expression("chrome.cast.Error");
    assert_eq!(r, None);
}
