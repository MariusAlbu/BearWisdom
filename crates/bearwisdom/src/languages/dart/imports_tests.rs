use super::super::extract;
use crate::types::EdgeKind;

#[test]
fn import_directive_produces_import_ref() {
    let src = "import 'dart:core';\nimport 'package:flutter/material.dart';\n";
    let r = extract::extract(src);
    let imports: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .collect();
    assert!(!imports.is_empty(), "expected import refs");
}

#[test]
fn plain_whole_library_import_is_wildcard() {
    // A bare `import 'uri';` — no `as`, no `show`/`hide` — brings every
    // declaration into unqualified scope: the wildcard sentinel target, the
    // `package:` URI's bare PACKAGE identity on `module` (not the library
    // file's stem) — `WildcardMatch::PackageRoot` compares it against a
    // candidate's external `ext:<lang>:<pkg>/…` segment.
    let src = "import 'package:flutter/material.dart';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert_eq!(imp.module.as_deref(), Some("flutter"));
}

#[test]
fn dart_scheme_wildcard_import_carries_library_name() {
    // `dart:async` has no `/` segment — the whole post-scheme string is the
    // package identity, matching the Dart-SDK ecosystem's `ext:dart-sdk:
    // async/…` virtual-path segment.
    let src = "import 'dart:async';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert_eq!(imp.module.as_deref(), Some("async"));
}

#[test]
fn relative_wildcard_import_falls_back_to_file_stem() {
    // A schemeless (project-relative) wildcard URI carries no package
    // identity — `module` falls back to the bare library stem, same as
    // before, so `WildcardMatch::PackageRoot`'s internal-candidate fallback
    // still lines up against a same-project file's basename.
    let src = "import 'widgets.dart';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert_eq!(imp.module.as_deref(), Some("widgets"));
}

#[test]
fn prefixed_import_is_not_wildcard() {
    // `import '...' as i0;` routes through the library-prefix mechanism —
    // it must never open the bare scope.
    let src = "import 'package:drift/drift.dart' as i0;\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_ne!(imp.target_name, "*");
    assert_eq!(imp.module.as_deref(), Some("package:drift/drift.dart"));
}

#[test]
fn show_restricted_import_is_not_wildcard() {
    // `show X, Y` limits the brought-in names to its list — not a wildcard.
    let src = "import 'package:flutter/material.dart' show Widget, State;\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_ne!(imp.target_name, "*");
}

#[test]
fn hide_restricted_import_is_approximated_as_wildcard() {
    // `hide X` still brings in every OTHER declaration; approximated here as
    // a full wildcard rather than tracking the excluded set.
    let src = "import 'package:flutter/material.dart' hide Widget;\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
}

#[test]
fn plain_export_emits_wildcard_reexport() {
    // A plain `export '...';` — no `show`/`hide` — re-exports every
    // declaration under the wildcard sentinel, tagged `is_reexport` so the
    // gate in `build_file_context` never treats it as opening the
    // DECLARING file's own scope. `module` keeps the raw URI (unreduced) —
    // the re-export ladder resolves it as a real specifier.
    let src = "export 'package:flutter/material.dart';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert!(imp.is_reexport);
    assert_eq!(imp.module.as_deref(), Some("package:flutter/material.dart"));
}

#[test]
fn relative_export_keeps_raw_specifier() {
    // A relative export's `module` is the literal specifier text, not a
    // package identity or file stem — the reexport ladder joins it against
    // the exporting file's own directory.
    let src = "export './sign_in_bloc.dart';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert!(imp.is_reexport);
    assert_eq!(imp.module.as_deref(), Some("./sign_in_bloc.dart"));
}

#[test]
fn show_restricted_export_emits_named_reexports() {
    // `show X, Y` limits a re-export to those names — one is_reexport ref
    // per shown name, target_name = the shown identifier, mirroring how a
    // named `export { X } from '...'` reexport ref is shaped.
    let src = "export 'models.dart' show User, Post;\n";
    let r = extract::extract(src);
    let names: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports && r.is_reexport)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(names, vec!["User", "Post"]);
    assert!(r
        .refs
        .iter()
        .all(|r| !r.is_reexport || r.module.as_deref() == Some("models.dart")));
}

#[test]
fn hide_restricted_export_is_approximated_as_wildcard_reexport() {
    // `hide X` still re-exports every OTHER declaration; approximated as a
    // full wildcard re-export rather than tracking the excluded set — same
    // approximation the import path uses for `hide`.
    let src = "export 'package:flutter/material.dart' hide Widget;\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert!(imp.is_reexport);
}
