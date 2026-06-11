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
