// =============================================================================
// c_lang/type_refs_tests.rs — macro-qualifier TypeRef suppression
// =============================================================================

use std::io::Write;

use super::extract::extract_with_file;
use super::macro_catalog::_reset_cache_for_test;
use crate::types::EdgeKind;

/// Write `name` with `content` into `dir` and return its absolute path string.
fn write_file(dir: &std::path::Path, name: &str, content: &str) -> String {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path.to_string_lossy().into_owned()
}

/// A calling-convention / export-qualifier macro discovered in a sibling header
/// is dropped from the TypeRef sweep — tree-sitter parses the leading macro
/// token as a `type_identifier`, but the macro expands to a linkage specifier,
/// not a type. A genuine type in the same leading position still emits.
#[test]
fn macro_qualifier_suppressed_real_type_kept() {
    _reset_cache_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();

    // Header in the same directory as the translation unit defines the export
    // macro; the catalog walk picks it up.
    write_file(dir, "api.h", "#define LUA_API __declspec(dllexport)\n");

    // `LUA_API` leads the declaration as a macro qualifier; `MyStruct` is a real
    // type used as a return type in the same leading position.
    let src = "struct MyStruct { int x; };\n\
               LUA_API int foo(int n) { return n; }\n\
               MyStruct bar(int n) { return MyStruct(); }\n";
    let cpp_path = write_file(dir, "unit.cpp", src);

    let result = extract_with_file(src, &cpp_path, "cpp");

    let type_ref_names: Vec<&str> = result
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();

    assert!(
        !type_ref_names.contains(&"LUA_API"),
        "macro qualifier LUA_API must not emit a TypeRef; got {type_ref_names:?}"
    );
    assert!(
        type_ref_names.contains(&"MyStruct"),
        "real type MyStruct must still emit a TypeRef; got {type_ref_names:?}"
    );
}

/// Without a catalog entry, the same leading token emits a TypeRef — proves the
/// suppression is driven by catalog membership, not a hardcoded name.
#[test]
fn unknown_qualifier_without_catalog_still_emits() {
    _reset_cache_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();

    // No header defines WINAPI here, so the catalog has no entry for it.
    let src = "WINAPI int foo(int n) { return n; }\n";
    let cpp_path = write_file(dir, "unit.cpp", src);

    let result = extract_with_file(src, &cpp_path, "cpp");

    let emits_winapi = result
        .refs
        .iter()
        .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "WINAPI");
    assert!(
        emits_winapi,
        "WINAPI with no catalog entry should emit a TypeRef (suppression is catalog-driven)"
    );
}

/// A reserved control-flow keyword that error recovery surfaces as a
/// `type_identifier` must never become a TypeRef. A bare `else x;` makes
/// tree-sitter-cpp parse `else` as the declaration's type token; the sweep's
/// keyword guard drops it.
#[test]
fn reserved_keyword_as_type_identifier_emits_no_type_ref() {
    let src = "else x;\n";
    let result = super::extract::extract(src, "cpp");

    let emits_else = result
        .refs
        .iter()
        .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "else");
    assert!(
        !emits_else,
        "reserved keyword `else` must not emit a TypeRef; got {:?}",
        result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect::<Vec<_>>()
    );
}

/// A plain declaration `MyStruct x;` still emits a TypeRef for `MyStruct` —
/// the keyword guard only suppresses reserved words.
#[test]
fn real_type_in_declaration_still_emits_type_ref() {
    let src = "MyStruct x;\n";
    let result = super::extract::extract(src, "cpp");

    let emits_mystruct = result
        .refs
        .iter()
        .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "MyStruct");
    assert!(
        emits_mystruct,
        "real type MyStruct must emit a TypeRef in a plain declaration"
    );
}

/// Collect every `TypeRef` target name emitted for `src`, extracted with
/// `file_path` so the macro catalog walks `file_path`'s sibling headers.
fn type_ref_names(src: &str, file_path: &str, lang: &str) -> Vec<String> {
    extract_with_file(src, file_path, lang)
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.clone())
        .collect()
}

/// A leading qualifier macro that stole the type slot leaves the real type in
/// an ERROR sibling. Both the macro in return-type position (`PERL_CALLCONV`)
/// and the threading-context macro in parameter position (`pTHX`) are
/// suppressed when a sibling header `#define`s them with an attribute/empty
/// body — neither can ever be a type.
#[test]
fn callconv_and_context_macros_suppressed_with_empty_define() {
    _reset_cache_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();

    // Both macros are defined empty in a sibling header — the dominant form.
    write_file(dir, "perl_api.h", "#define PERL_CALLCONV\n#define pTHX\n");

    let src = "PERL_CALLCONV void Perl_foo(pTHX);\n";
    let c_path = write_file(dir, "unit.c", src);

    let names = type_ref_names(src, &c_path, "c");
    assert!(
        !names.iter().any(|n| n == "PERL_CALLCONV"),
        "calling-convention macro must not emit a TypeRef; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "pTHX"),
        "context macro must not emit a TypeRef; got {names:?}"
    );
}

/// The position rule alone — no catalog entry — suppresses a leading qualifier
/// macro when the real type was demoted to an ERROR sibling
/// (`SOMEMACRO int foo(...)`). This proves the structural signal is independent
/// of the catalog.
#[test]
fn leading_macro_with_error_sibling_suppressed_without_catalog() {
    _reset_cache_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    // No header defines SOMEMACRO — the catalog is empty for it.
    let src = "SOMEMACRO int foo(int n);\n";
    let c_path = write_file(dir, "unit.c", src);

    let names = type_ref_names(src, &c_path, "c");
    assert!(
        !names.iter().any(|n| n == "SOMEMACRO"),
        "leading macro with an ERROR-demoted real type must be suppressed by \
         position alone; got {names:?}"
    );
}

/// The position rule is gated to the leading declaration/parameter type slot.
/// A real type in a plain declaration (`MyStruct x;`) has no ERROR sibling, so
/// it still emits — the rule never fires on well-formed code.
#[test]
fn position_rule_does_not_suppress_plain_declaration() {
    _reset_cache_for_test();
    let src = "MyStruct x;\n";
    let names: Vec<String> = super::extract::extract(src, "c")
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "MyStruct"),
        "plain declaration type must survive the position rule; got {names:?}"
    );
}

/// `struct foo bar;` carries the type via a `struct_specifier`, not a leading
/// `type_identifier`, so the position rule cannot touch it — the struct tag is
/// unaffected.
#[test]
fn position_rule_does_not_touch_struct_specifier_decl() {
    _reset_cache_for_test();
    let src = "struct foo bar;\n";
    let result = super::extract::extract(src, "c");
    // No bogus suppression: the declaration extracts a Variable `bar`.
    assert!(
        result.symbols.iter().any(|s| s.name == "bar"),
        "struct-tag declaration must still extract its variable"
    );
}

/// A type used inside a template argument list is never in a leading
/// declaration slot, so the position rule leaves it alone.
#[test]
fn position_rule_does_not_touch_template_arguments() {
    _reset_cache_for_test();
    let src = "Vec<MyElem> v;\n";
    let names: Vec<String> = super::extract::extract(src, "cpp")
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "MyElem"),
        "template argument type must still emit a TypeRef; got {names:?}"
    );
}

/// Collect every `TypeRef` target name emitted for `src` extracted with the
/// plain `extract` entry (no sibling-header catalog) — exercises the position
/// rule in isolation.
fn type_ref_names_plain(src: &str, lang: &str) -> Vec<String> {
    super::extract::extract(src, lang)
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.clone())
        .collect()
}

/// SAL parameter annotations whose real type follows in the *declarator* slot —
/// `_In_ HANDLE h`, `IN HANDLE h`, `__in HANDLE h` — push the trailing parameter
/// name into an ERROR sibling of the whole `parameter_declaration`. The position
/// rule climbs to the parameter level and suppresses the annotation token in
/// both C and C++.
#[test]
fn sal_param_annotation_with_trailing_name_error_suppressed() {
    _reset_cache_for_test();
    for (lang, annot) in [
        ("c", "_In_"),
        ("cpp", "_In_"),
        ("c", "IN"),
        ("cpp", "IN"),
        ("c", "__in"),
    ] {
        let src = format!("void f({annot} HANDLE h);\n");
        let names = type_ref_names_plain(&src, lang);
        assert!(
            !names.iter().any(|n| n == annot),
            "SAL annotation {annot} (param, trailing-name ERROR) must not emit a \
             TypeRef in {lang}; got {names:?}"
        );
    }
}

/// SAL parameter annotations whose real type is demoted into an ERROR sibling
/// directly after the type slot — `_Inout_ int* p`, `_Out_ char* q`,
/// `OUT DWORD* n`, `__out DWORD* n` — are suppressed by the same-level ERROR
/// fingerprint the original rule already covered.
#[test]
fn sal_param_annotation_with_demoted_type_error_suppressed() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("void f(_Inout_ int* p, _Out_ char* q);\n", "c");
    for annot in ["_Inout_", "_Out_"] {
        assert!(
            !names.iter().any(|n| n == annot),
            "SAL annotation {annot} (demoted-type ERROR) must not emit a TypeRef; got {names:?}"
        );
    }

    let names = type_ref_names_plain("void f(OUT DWORD* n, __out DWORD* m);\n", "cpp");
    for annot in ["OUT", "__out"] {
        assert!(
            !names.iter().any(|n| n == annot),
            "calling-convention annotation {annot} must not emit a TypeRef; got {names:?}"
        );
    }
}

/// `EXTERN_C HRESULT __stdcall Foo(...)` demotes `HRESULT __stdcall` into an
/// ERROR sibling of the declaration; the leading linkage macro `EXTERN_C` is
/// suppressed. `__stdcall` itself parses as an `ms_call_modifier`, never a
/// `type_identifier`, so it never reaches the sweep.
#[test]
fn extern_c_linkage_macro_in_return_position_suppressed() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("EXTERN_C HRESULT __stdcall Foo(int x);\n", "cpp");
    assert!(
        !names.iter().any(|n| n == "EXTERN_C"),
        "linkage macro EXTERN_C must not emit a TypeRef; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "__stdcall"),
        "calling-convention keyword __stdcall must never reach the sweep; got {names:?}"
    );
}

/// A bare calling-convention specifier in a well-formed declaration
/// (`HRESULT __stdcall Foo(...)`) parses cleanly: `HRESULT` is the real type and
/// emits, `__stdcall` is an `ms_call_modifier`. The position rule never fires
/// because there is no ERROR sibling.
#[test]
fn clean_ms_call_modifier_keeps_real_return_type() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("HRESULT __stdcall Foo(int x);\n", "c");
    assert!(
        names.iter().any(|n| n == "HRESULT"),
        "real return type HRESULT must still emit a TypeRef; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "__stdcall"),
        "__stdcall must never emit a TypeRef; got {names:?}"
    );
}

/// A SAL annotation carrying a count argument (`_In_reads_(cbLen)`) parses as a
/// `macro_type_specifier` whose parenthesised count expression is captured as a
/// `type_descriptor`. The count token (`cbLen`) must not leak as a TypeRef, and
/// the demoted real parameter type (`char`) must not be misattributed to the
/// annotation.
#[test]
fn sal_macro_type_specifier_count_argument_suppressed() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("void f(_In_reads_(cbLen) char* buf, int cbLen);\n", "c");
    assert!(
        !names.iter().any(|n| n == "cbLen"),
        "SAL count argument cbLen must not emit a TypeRef; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "_In_reads_"),
        "SAL annotation macro _In_reads_ must not emit a TypeRef; got {names:?}"
    );
}

/// PRECISION: a real user type in the same leading parameter slot
/// (`void f(Foo h)`) has no ERROR sibling, so the parameter-level climb never
/// fires — `Foo` still emits a TypeRef.
#[test]
fn real_type_in_param_slot_not_suppressed() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("void f(Foo h);\n", "c");
    assert!(
        names.iter().any(|n| n == "Foo"),
        "real parameter type Foo must still emit a TypeRef; got {names:?}"
    );
}

/// PRECISION: a `const Foo x;` declaration carries the real type past a
/// `type_qualifier`, with no ERROR sibling — the rule leaves `Foo` alone.
#[test]
fn const_qualified_real_type_not_suppressed() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("const Foo x;\n", "c");
    assert!(
        names.iter().any(|n| n == "Foo"),
        "const-qualified real type Foo must still emit a TypeRef; got {names:?}"
    );
}

/// PRECISION: an ERROR sibling that holds only punctuation (`Foo x }};`) is not
/// the demotion fingerprint — the rule requires the ERROR to carry an
/// identifier, so the real type `Foo` still emits even though the declaration
/// has an ERROR child.
#[test]
fn punctuation_only_error_sibling_does_not_suppress_real_type() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("Foo x }} ;\n", "c");
    assert!(
        names.iter().any(|n| n == "Foo"),
        "real type Foo must survive a punctuation-only ERROR sibling; got {names:?}"
    );
}

/// PRECISION: the D-M2 keyword guard still holds — `else x;` makes tree-sitter
/// parse `else` as the type token, but the reserved keyword is dropped and the
/// SAL extension never resurrects it.
#[test]
fn reserved_keyword_still_dropped_after_sal_extension() {
    _reset_cache_for_test();
    let names = type_ref_names_plain("else x;\n", "cpp");
    assert!(
        !names.iter().any(|n| n == "else"),
        "reserved keyword else must not emit a TypeRef; got {names:?}"
    );
}
