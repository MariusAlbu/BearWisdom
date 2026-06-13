// =============================================================================
// c_lang/declarations_tests.rs  —  return-type-as-function-name guard
// =============================================================================

use super::*;
use crate::types::SymbolKind;

/// A function/method symbol whose name equals its own return-type token is a
/// COM/calling-convention misparse artifact and must never be emitted.
#[test]
fn function_named_after_its_return_type_is_skipped() {
    let src = r#"
HRESULT HRESULT() {
    return 0;
}
"#;
    let r = extract::extract(src, "cpp");
    assert!(
        !r.symbols
            .iter()
            .any(|s| s.name == "HRESULT"
                && matches!(s.kind, SymbolKind::Function | SymbolKind::Method)),
        "function whose name == return type must be suppressed: {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, &s.kind))
            .collect::<Vec<_>>()
    );
}

/// The guard is exact-token only — a legitimate function with a distinct name
/// is emitted normally.
#[test]
fn normal_function_is_not_skipped() {
    let src = r#"
int foo() {
    return 0;
}
"#;
    let r = extract::extract(src, "cpp");
    let foo = r.symbols.iter().find(|s| s.name == "foo").expect("foo");
    assert_eq!(foo.kind, SymbolKind::Function);
}

/// COM vtbl declarator (`HRESULT ( STDMETHODCALLTYPE *QueryInterface )( ... )`):
/// the unexpanded `STDMETHODCALLTYPE` macro makes tree-sitter parse this as a
/// function declarator NAMED `HRESULT` with an EMPTY return-type field — so the
/// name==return-type guard cannot fire (the return type isn't `HRESULT`, it's
/// absent). Recovering the real member name (`QueryInterface`) and suppressing
/// the `HRESULT` name requires recognizing the parenthesized function-pointer-
/// member shape — a separate parse-tree cut, not the return-type==name guard.
#[test]
#[ignore = "needs COM vtbl function-pointer-member shape recovery; the guard only catches return-type==name"]
fn com_vtbl_declarator_does_not_emit_returntype_function() {
    let src = r#"
typedef struct IUnknownVtbl {
    HRESULT ( STDMETHODCALLTYPE *QueryInterface )( IUnknown *This, REFIID riid, void **ppvObject );
    ULONG ( STDMETHODCALLTYPE *AddRef )( IUnknown *This );
} IUnknownVtbl;
"#;
    let r = extract::extract(src, "cpp");
    assert!(
        !r.symbols
            .iter()
            .any(|s| (s.name == "HRESULT" || s.name == "ULONG")
                && matches!(s.kind, SymbolKind::Function | SymbolKind::Method)),
        "return-type token leaked as a function/method symbol: {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, &s.kind))
            .collect::<Vec<_>>()
    );
}

/// Look up a symbol by name and assert it exists with the given kind.
fn assert_symbol_kind(r: &crate::types::ExtractionResult, name: &str, kind: SymbolKind) {
    let found = r.symbols.iter().find(|s| s.name == name);
    assert!(
        matches!(found, Some(s) if s.kind == kind),
        "expected `{name}` as {kind:?}, got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, &s.kind))
            .collect::<Vec<_>>()
    );
}

/// A GCC vector-attribute typedef (`typedef __attribute__((...)) T Alias;`)
/// splits into a `type_definition` that strands the `typedef` keyword plus a
/// sibling `declaration` holding the real `T Alias;`. The alias must surface as
/// a TypeAlias, not the Variable the sibling declaration would otherwise emit.
#[test]
fn typedef_with_leading_attribute_emits_type_alias() {
    let src = "typedef __attribute__((neon_vector_type(4))) int32_t int32x4_t;\n";
    let r = extract::extract(src, "c");
    assert_symbol_kind(&r, "int32x4_t", SymbolKind::TypeAlias);
    assert!(
        !r.symbols
            .iter()
            .any(|s| s.name == "int32x4_t" && s.kind == SymbolKind::Variable),
        "alias must not also surface as a Variable"
    );
}

/// `__attribute__((aligned(N)))` before a `struct foo` source type — the alias
/// (`bar`) is still a TypeAlias; the referenced `struct foo` is a separate
/// symbol the visitor emits independently.
#[test]
fn typedef_with_attribute_before_struct_source_emits_type_alias() {
    let src = "typedef __attribute__((aligned(16))) struct foo bar;\n";
    let r = extract::extract(src, "c");
    assert_symbol_kind(&r, "bar", SymbolKind::TypeAlias);
}

/// `__declspec`-prefixed typedef (MSVC form) routes through the same
/// split-sibling path under the C++ grammar.
#[test]
fn typedef_with_declspec_emits_type_alias() {
    let src = "typedef __declspec(align(16)) int aligned_int;\n";
    let r = extract::extract(src, "cpp");
    assert_symbol_kind(&r, "aligned_int", SymbolKind::TypeAlias);
}

/// Trailing-attribute (GCC postfix) form stays on the `type_definition` path:
/// the alias is a direct `type_identifier` child and the attribute trails as an
/// `attribute_specifier` sibling, so the existing typedef handler emits it.
#[test]
fn typedef_with_trailing_attribute_emits_type_alias() {
    let src = "typedef int my_aligned __attribute__((aligned(16)));\n";
    let r = extract::extract(src, "c");
    assert_symbol_kind(&r, "my_aligned", SymbolKind::TypeAlias);
}

/// A plain attribute-free typedef still resolves to a TypeAlias (regression).
#[test]
fn plain_typedef_still_emits_type_alias() {
    let src = "typedef int32_t myint;\n";
    let r = extract::extract(src, "c");
    assert_symbol_kind(&r, "myint", SymbolKind::TypeAlias);
}

/// A non-typedef declaration is unaffected by the attribute-typedef handling —
/// `x` is a Variable, never a TypeAlias.
#[test]
fn plain_declaration_still_emits_variable() {
    let src = "int32_t x;\n";
    let r = extract::extract(src, "c");
    assert_symbol_kind(&r, "x", SymbolKind::Variable);
    assert!(
        !r.symbols
            .iter()
            .any(|s| s.name == "x" && s.kind == SymbolKind::TypeAlias),
        "plain declaration must not become a TypeAlias"
    );
}
