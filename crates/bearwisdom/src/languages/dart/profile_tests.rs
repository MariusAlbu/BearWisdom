use super::DART_PROFILE;
use crate::type_checker::profile::language_profile::{
    ImportModulePath, KindCompatibility, WildcardMatch,
};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn dart_profile_identity() {
    assert_eq!(DART_PROFILE.id, "dart");
    assert_eq!(DART_PROFILE.self_keywords, &["this", "super"]);
}

#[test]
fn dart_implements_accepts_class_and_interface() {
    let t = DART_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Implements,
        SymbolKind::Class
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Implements,
        SymbolKind::Interface
    ));
}

#[test]
fn dart_async_wrappers_contain_future_and_stream() {
    assert!(DART_PROFILE.async_wrappers.contains(&"Future"));
    assert!(DART_PROFILE.async_wrappers.contains(&"Stream"));
}

#[test]
fn dart_wildcard_import_carries_module_path() {
    // `import_module_path` must reach `FromModuleField` for a wildcard
    // import ref's `module` to reach `ImportEntry.module_path` at all —
    // `None` leaves every dart import entry's `module_path` empty and the
    // wildcard-import rung filters those out before it ever inspects a
    // candidate.
    assert_eq!(DART_PROFILE.import_module_path, ImportModulePath::FromModuleField);
}

#[test]
fn dart_wildcard_match_is_file_stem() {
    // Dart top-level declarations carry no namespace prefix in their
    // qualified name, so `QnameUnder` can never match a whole-library
    // import; only `FileStem` can line up a wildcard's module stem against
    // a candidate's declaring-file basename.
    assert_eq!(
        DART_PROFILE.wildcard_match,
        WildcardMatch::FileStem { underscore_prefix: false }
    );
}
