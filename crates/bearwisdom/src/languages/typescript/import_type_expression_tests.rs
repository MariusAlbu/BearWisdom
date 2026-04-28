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

use super::parse_import_type_expression;

#[test]
fn single_quoted_module_with_simple_type() {
    let r = parse_import_type_expression("import('node:stream').Readable");
    assert_eq!(r, Some(("node:stream".to_string(), "Readable".to_string())));
}

#[test]
fn double_quoted_module_with_simple_type() {
    let r = parse_import_type_expression("import(\"typescript\").Diagnostic");
    assert_eq!(r, Some(("typescript".to_string(), "Diagnostic".to_string())));
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
