use super::PHP_PROFILE;
use crate::type_checker::profile::language_profile::{ChainQualification, KindCompatibility};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn php_profile_identity() {
    assert_eq!(PHP_PROFILE.id, "php");
    assert_eq!(PHP_PROFILE.qname_separator, "\\");
}

#[test]
fn php_chain_qualification_is_same_package_and_imports() {
    // Same-namespace + `use`-statement qualification through the engine walker.
    assert_eq!(
        PHP_PROFILE.chain_qualification,
        ChainQualification::SamePackageAndImports
    );
}

#[test]
fn php_profile_self_keywords_cover_receiver_forms() {
    assert!(PHP_PROFILE.self_keywords.contains(&"$this"));
    assert!(PHP_PROFILE.self_keywords.contains(&"self"));
    assert!(PHP_PROFILE.self_keywords.contains(&"static"));
    assert!(PHP_PROFILE.self_keywords.contains(&"parent"));
}

#[test]
fn php_calls_accepts_function_method_constructor() {
    let t = PHP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Constructor
    ));
}

#[test]
fn php_implements_accepts_interface_only() {
    let t = PHP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Implements,
        SymbolKind::Interface
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Implements,
        SymbolKind::Class
    ));
}

#[test]
fn php_instantiates_accepts_class_only() {
    let t = PHP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Instantiates,
        SymbolKind::Class
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Instantiates,
        SymbolKind::Interface
    ));
}

#[test]
fn php_builtin_skip_declines_language_constructs_not_user_functions() {
    // PHP language constructs (isset/empty/unset/echo/print/list/eval/exit/die)
    // are reserved keywords, not callable functions — decline them before the
    // ladder so a same-named project symbol can never bind. A user-defined
    // function is NOT in the construct set, so it resolves through the ladder.
    let skip = PHP_PROFILE
        .builtin_skip
        .expect("php builtin_skip set for language constructs");
    assert!(skip("isset"));
    assert!(skip("empty"));
    assert!(skip("unset"));
    assert!(skip("echo"));
    assert!(skip("print"));
    assert!(skip("list"));
    assert!(skip("eval"));
    assert!(skip("exit"));
    assert!(skip("die"));
    assert!(!skip("my_user_function"));
}
