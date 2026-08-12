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
    // library's bare stem on `module`.
    let src = "import 'package:flutter/material.dart';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_eq!(imp.target_name, "*");
    assert_eq!(imp.module.as_deref(), Some("material"));
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
fn export_directive_is_never_wildcard() {
    // `export` never opens the declaring file's own scope, regardless of
    // its combinators.
    let src = "export 'package:flutter/material.dart';\n";
    let r = extract::extract(src);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("import ref");
    assert_ne!(imp.target_name, "*");
}
