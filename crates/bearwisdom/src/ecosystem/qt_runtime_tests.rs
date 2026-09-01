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
    fs::write(
        include.join("QtWidgets/QWidget"),
        "#include \"qwidget.h\"\n",
    )
    .unwrap();

    (tmp, include)
}

#[test]
fn qt_locator_finds_install_via_standard_qtdir_env() {
    let _env = crate::ecosystem::QTDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // `QTDIR` is the standard env var the Qt toolchain/qmake exports — it
    // names the install root, and discovery drills into its `include/`.
    // No BearWisdom-private override is consulted; standard discovery alone
    // surfaces the install.
    let (_tmp, include) = fixture_qt_include();
    let parent = include.parent().unwrap();
    let prior = std::env::var_os("QTDIR");
    std::env::set_var("QTDIR", parent);
    let roots = discover_qt_include();
    match prior {
        Some(p) => std::env::set_var("QTDIR", p),
        None => std::env::remove_var("QTDIR"),
    }
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
        idx.locate("QtCore/private/qobject_p.h", "qobject_p.h")
            .is_none(),
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
    assert!(
        missing.is_none(),
        "must return None for a header not in the tree"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn qt_windows_probe_finds_standard_install_root() {
    let _env = crate::ecosystem::QTDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The official Qt online installer lays the SDK out as
    // `C:/Qt/<version>/<kit>/include`. When such an install exists on the
    // host, the Windows autodetect probe must surface its `include/` dir —
    // no QTDIR / env override required.
    let std_root = std::path::Path::new("C:/Qt");
    if !std_root.is_dir() {
        return; // no Qt install on this host — nothing to assert
    }
    // Find at least one `C:/Qt/<ver>/<kit>/include` on disk to compare against.
    let mut expected: Option<std::path::PathBuf> = None;
    if let Ok(vers) = fs::read_dir(std_root) {
        for ver in vers.flatten().filter(|e| e.path().is_dir()) {
            if let Ok(kits) = fs::read_dir(ver.path()) {
                for kit in kits.flatten().filter(|e| e.path().is_dir()) {
                    let inc = kit.path().join("include");
                    if inc.is_dir() {
                        expected = Some(inc);
                        break;
                    }
                }
            }
            if expected.is_some() {
                break;
            }
        }
    }
    let Some(expected) = expected else {
        return; // C:/Qt exists but holds no `<ver>/<kit>/include` — skip
    };
    let found = autodetect_qt_include_dirs();
    assert!(
        found.iter().any(|p| p == &expected),
        "Windows probe missed standard install root {expected:?}; got {found:?}"
    );
}

#[test]
fn qt_locator_returns_only_real_on_disk_roots() {
    // Fixture-free scenario: discovery probes standard env vars and install
    // paths only. We can't assert empty unconditionally because the host
    // might have Qt installed in a default location. We CAN assert that none
    // of the roots are bogus — every returned dep root must point at an
    // existing dir.
    let roots = discover_qt_include();
    for r in &roots {
        assert!(
            r.root.is_dir(),
            "discovered root must exist on disk: {:?}",
            r.root
        );
    }
}
