use super::*;
use std::fs;
use tempfile::TempDir;

/// Create a synthetic Qt install layout under `root/include/` covering both
/// the camelcase class wrapper form (`QObject`) and the real header form
/// (`qobject.h`) for two modules. Returns the include directory.
fn fixture_qt_include() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let include = tmp.path().join("include");
    fs::create_dir_all(include.join("QtCore")).unwrap();
    fs::create_dir_all(include.join("QtCore").join("private")).unwrap();
    fs::create_dir_all(include.join("QtWidgets")).unwrap();

    // QtCore — real headers
    fs::write(include.join("QtCore/qobject.h"), "class QObject {};\n").unwrap();
    fs::write(include.join("QtCore/qstring.h"), "class QString {};\n").unwrap();
    // QtCore — camelcase wrappers
    fs::write(include.join("QtCore/QObject"), "#include \"qobject.h\"\n").unwrap();
    fs::write(include.join("QtCore/QString"), "#include \"qstring.h\"\n").unwrap();
    // Private dir — must NOT be indexed
    fs::write(include.join("QtCore/private/qobject_p.h"), "// internal\n").unwrap();

    // QtWidgets
    fs::write(include.join("QtWidgets/qwidget.h"), "class QWidget {};\n").unwrap();
    fs::write(include.join("QtWidgets/QWidget"), "#include \"qwidget.h\"\n").unwrap();

    (tmp, include)
}

#[test]
fn qt_locator_finds_install_via_explicit_env_var() {
    let (_tmp, include) = fixture_qt_include();
    // Use parent dir as BEARWISDOM_QT_DIR — locator should drill into `include/`.
    let parent = include.parent().unwrap();
    std::env::set_var("BEARWISDOM_QT_DIR", parent);
    let roots = discover_qt_include();
    std::env::remove_var("BEARWISDOM_QT_DIR");
    assert!(
        roots.iter().any(|r| r.root == include),
        "expected discovery to find include dir; got roots={:?}",
        roots.iter().map(|r| &r.root).collect::<Vec<_>>()
    );
}

#[test]
fn qt_index_registers_camelcase_wrappers_and_real_headers() {
    let (_tmp, include) = fixture_qt_include();
    let dep = make_root(&include);
    let idx = build_qt_header_index(&[dep]);

    // The real headers must be registered under their relative path AND
    // their basename, so both `#include <QtCore/qobject.h>` and
    // `#include <qobject.h>` resolve.
    assert!(
        idx.locate("QtCore/qobject.h", "QtCore/qobject.h").is_some(),
        "missing relative-path entry for QtCore/qobject.h"
    );
    assert!(
        idx.locate("QtCore/qobject.h", "qobject.h").is_some(),
        "missing basename entry for qobject.h via QtCore module path"
    );
    // Camelcase wrapper as a bare include (`#include <QObject>`) — the C
    // extractor will emit target=QObject, module=QObject for that.
    assert!(
        idx.locate("QObject", "QObject").is_some(),
        "missing bare-name entry for QObject (`#include <QObject>` form)"
    );
    // Same for the second module — make sure cross-module headers are present.
    assert!(
        idx.locate("QWidget", "QWidget").is_some(),
        "missing bare-name entry for QWidget"
    );
}

#[test]
fn qt_index_skips_private_subdirs() {
    let (_tmp, include) = fixture_qt_include();
    let dep = make_root(&include);
    let idx = build_qt_header_index(&[dep]);

    // qobject_p.h lives in QtCore/private/ and must not be indexed under any form.
    assert!(
        idx.locate("QtCore/private/qobject_p.h", "qobject_p.h").is_none(),
        "private-dir headers must be filtered"
    );
    assert!(
        idx.locate("qobject_p.h", "qobject_p.h").is_none(),
        "private-dir basename must be filtered"
    );
}

#[test]
fn qt_resolve_header_finds_by_relative_or_basename() {
    let (_tmp, include) = fixture_qt_include();
    let dep = make_root(&include);

    let by_rel = resolve_qt_header(&dep, "QtCore/qobject.h");
    assert!(by_rel.is_some(), "must resolve by relative path");

    let by_basename = resolve_qt_header(&dep, "qstring.h");
    assert!(by_basename.is_some(), "must resolve by basename fallback");

    let missing = resolve_qt_header(&dep, "nonexistent.h");
    assert!(missing.is_none(), "must return None for a header not in the tree");
}

#[test]
fn qt_locator_returns_empty_when_no_install_present() {
    // Make sure NO env override is set and probe — fixture-free scenario.
    let prior = std::env::var_os("BEARWISDOM_QT_DIR");
    std::env::remove_var("BEARWISDOM_QT_DIR");
    let roots = discover_qt_include();
    if let Some(p) = prior { std::env::set_var("BEARWISDOM_QT_DIR", p); }
    // We can't assert empty unconditionally because the host might have Qt
    // installed in a default location. We CAN assert that none of the roots
    // are bogus — every returned dep root must point at an existing dir.
    for r in &roots {
        assert!(r.root.is_dir(), "discovered root must exist on disk: {:?}", r.root);
    }
}
