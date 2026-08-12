use super::PASCAL_PROFILE;
use crate::type_checker::profile::language_profile::{
    ImportModulePath, KindCompatibility, NameNormalization, WildcardMatch,
};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn pascal_calls_row_accepts_type_constructors_without_dropping_functions() {
    let t = PASCAL_PROFILE.kind_compatible_table;
    // `TFoo(x)` type-cast / record-constructor binds to the type declaration.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::TypeAlias
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Struct
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Class
    ));
    // A real function call still resolves to the function (no regression).
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    // A Calls ref must still not bind to a value.
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
}

#[test]
fn pascal_profile_identity_and_shadow_mode() {
    assert_eq!(PASCAL_PROFILE.id, "pascal");
    assert_eq!(PASCAL_PROFILE.self_keywords, &["Self"]);
}

#[test]
fn pascal_builtin_skip_drains_casts_declines_project_declared_names() {
    // Type-cast/compiler-intrinsic names with zero project declarations
    // drain; names the Pascal reference corpus declares internally (an
    // RTL-compat shim, a GTK binding, a code-generated field-type class)
    // fall through to the ladder's normal lookup rungs instead.
    let skip = PASCAL_PROFILE.builtin_skip.expect("pascal builtin_skip set");
    assert!(skip("Integer"));
    assert!(skip("single")); // case-insensitive: Pascal identifiers fold case
    assert!(!skip("FreeAndNil"));
    assert!(!skip("Inc"));
    assert!(!skip("TObject"));
}

#[test]
fn pascal_case_folds_and_matches_units_by_file_stem() {
    // The former resolve_ref case-insensitive same-file match drains to the
    // case-folding NameNormalization; the wildcard unit-import match drains to
    // FileStem with the `{unit}_…` include-file probe.
    assert!(matches!(
        PASCAL_PROFILE.name_normalization,
        NameNormalization::Spec(spec) if spec.case_insensitive
    ));
    assert!(matches!(
        PASCAL_PROFILE.wildcard_match,
        WildcardMatch::FileStem {
            underscore_prefix: true
        }
    ));
}

#[test]
fn pascal_uses_clause_reaches_the_wildcard_rung() {
    // `WildcardMatch::FileStem` above is inert data unless `build_file_context`
    // actually marks a `uses` entry as a wildcard import. That gate is
    // `namespace_imports_are_wildcards` (a bare `uses X;` opens the whole unit,
    // same as C#'s plain `using X;`), and it requires `module_path` to be
    // populated — `extract_uses` sets `ExtractedRef::module`, so the profile
    // must read it via `FromModuleField` rather than leaving it unset.
    assert!(PASCAL_PROFILE.namespace_imports_are_wildcards);
    assert!(matches!(
        PASCAL_PROFILE.import_module_path,
        ImportModulePath::FromModuleField
    ));
}
