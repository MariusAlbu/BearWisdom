use super::DART_PROFILE;
use crate::type_checker::profile::language_profile::{
    ImportModulePath, KindCompatibility, WildcardMatch,
};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn dart_profile_identity() {
    assert_eq!(DART_PROFILE.id, "dart");
    assert_eq!(DART_PROFILE.receiver_spellings, &["this", "super"]);
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
    assert_eq!(
        DART_PROFILE.imports.import_module_path,
        ImportModulePath::FromModuleField
    );
}

#[test]
fn dart_wildcard_match_is_package_root() {
    // Dart top-level declarations carry no namespace prefix in their
    // qualified name, so `QnameUnder` can never match a whole-library
    // import. `PackageRoot` reaches through a barrel library
    // (`package:flutter/material.dart` re-exporting `src/widgets/
    // framework.dart`) by matching the wildcard's package identity against
    // a candidate's external package segment, falling back to the same
    // file-stem check `FileStem` used before it for relative imports.
    assert_eq!(
        DART_PROFILE.imports.wildcard_match,
        WildcardMatch::PackageRoot
    );
}

#[test]
fn dart_scopes_both_import_forms_to_workspace_packages() {
    // A `package:` URI names a sibling workspace package, whether the import
    // binds names explicitly or as a whole-library glob; both rungs are
    // reachable only while these gates are on.
    assert!(DART_PROFILE.imports.workspace_packages);
    assert!(DART_PROFILE.imports.wildcard_workspace_scope);
}

#[test]
fn dart_package_uri_is_a_bare_module_specifier() {
    // The workspace rungs require a BARE specifier; a `package:` URI must not
    // be read as a relative path (its second byte is not the drive-letter
    // colon that marks one).
    let policy = DART_PROFILE.source_module_path_policy("package:core_client/core_client.dart");
    assert!(policy.is_bare("package:core_client/core_client.dart"));
}
