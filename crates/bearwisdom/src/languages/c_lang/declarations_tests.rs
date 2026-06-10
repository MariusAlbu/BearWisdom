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
