// =============================================================================
// c_lang/macro_catalog_tests.rs — `#define` discovery + body-shape classification
// =============================================================================

use super::macro_catalog::{_catalog_from_source, _is_attribute_shaped_body};

// ---------------------------------------------------------------------------
// Directive parsing — whitespace between `#` and `define`
// ---------------------------------------------------------------------------

/// The C preprocessor permits whitespace between `#` and `define`. Headers that
/// indent directives by conditional-nesting depth (`#  define NAME`) are
/// common; the catalog must still capture those names.
#[test]
fn captures_define_with_space_after_hash() {
    let catalog = _catalog_from_source("#  define PERL_CALLCONV\n");
    assert!(
        catalog.by_name.contains_key("PERL_CALLCONV"),
        "`#  define` (space after #) must be captured"
    );
}

#[test]
fn captures_deeply_indented_define() {
    let catalog = _catalog_from_source("#      define EXTCONST extern\n");
    assert!(catalog.by_name.contains_key("EXTCONST"));
}

/// The canonical adjacent form still parses.
#[test]
fn captures_adjacent_define() {
    let catalog = _catalog_from_source("#define FOO 1\n");
    assert!(catalog.by_name.contains_key("FOO"));
}

/// `#defined` / `#definition` are not `#define` directives — a separator is
/// required after the keyword.
#[test]
fn rejects_define_without_separator() {
    let catalog = _catalog_from_source("#defined FOO\n#definition BAR\n");
    assert!(!catalog.by_name.contains_key("FOO"));
    assert!(!catalog.by_name.contains_key("BAR"));
}

// ---------------------------------------------------------------------------
// Body-shape classification — attribute/linkage/storage vs type alias
// ---------------------------------------------------------------------------

/// Empty body — the dominant qualifier form (`#define PERL_CALLCONV`).
#[test]
fn empty_body_is_attribute_shaped() {
    assert!(_is_attribute_shaped_body(""));
    assert!(_is_attribute_shaped_body("   "));
}

#[test]
fn attribute_and_declspec_bodies_are_attribute_shaped() {
    assert!(_is_attribute_shaped_body("__attribute__((noreturn))"));
    assert!(_is_attribute_shaped_body("__declspec(dllimport)"));
    assert!(_is_attribute_shaped_body("extern \"C\" __declspec(dllimport)"));
}

#[test]
fn linkage_and_storage_keyword_bodies_are_attribute_shaped() {
    assert!(_is_attribute_shaped_body("extern"));
    assert!(_is_attribute_shaped_body("static inline"));
    assert!(_is_attribute_shaped_body("__cdecl"));
    assert!(_is_attribute_shaped_body("__stdcall"));
    // Reserved-identifier convention specifier (`_System`, OS/2 linkage).
    assert!(_is_attribute_shaped_body("_System"));
}

/// A macro that aliases a real type is NOT attribute-shaped — its TypeRef must
/// survive so the name resolves to the alias.
#[test]
fn type_aliasing_bodies_are_not_attribute_shaped() {
    assert!(!_is_attribute_shaped_body("int"));
    assert!(!_is_attribute_shaped_body("unsigned long"));
    assert!(!_is_attribute_shaped_body("struct sockaddr"));
    // Pointer alias — the `*` token disqualifies it.
    assert!(!_is_attribute_shaped_body("void *"));
    // A SCREAMING_CASE identifier that isn't a known keyword could be a type.
    assert!(!_is_attribute_shaped_body("MyHandle"));
}

// ---------------------------------------------------------------------------
// is_attribute_macro — name lookup gated by body shape
// ---------------------------------------------------------------------------

/// `#define MyInt int` is a type alias: `is_attribute_macro` is false, so the
/// retain keeps its TypeRef.
#[test]
fn type_alias_macro_is_not_attribute_macro() {
    let catalog = _catalog_from_source("#define MyInt int\n");
    assert!(catalog.by_name.contains_key("MyInt"));
    assert!(
        !catalog.is_attribute_macro("MyInt"),
        "a type-aliasing macro must not classify as an attribute macro"
    );
}

/// `#define PERL_CALLCONV` (empty) is a qualifier: `is_attribute_macro` is true.
#[test]
fn empty_macro_is_attribute_macro() {
    let catalog = _catalog_from_source("#  define PERL_CALLCONV\n");
    assert!(catalog.is_attribute_macro("PERL_CALLCONV"));
}

/// When a name is defined across branches with both an attribute-shaped and a
/// type-shaped body, the attribute-shaped one wins — a qualifier reads as a
/// qualifier regardless of branch-walk order.
#[test]
fn attribute_shaped_body_wins_across_conditional_branches() {
    // Type-shaped body appears first, attribute-shaped (empty) second.
    let catalog = _catalog_from_source("#  define pTHX void\n#  define pTHX\n");
    assert!(
        catalog.is_attribute_macro("pTHX"),
        "an empty/attribute branch must override a type-shaped branch"
    );
}

/// An unknown name (no `#define`) is not an attribute macro.
#[test]
fn undefined_name_is_not_attribute_macro() {
    let catalog = _catalog_from_source("#define OTHER 1\n");
    assert!(!catalog.is_attribute_macro("WINAPI"));
}
