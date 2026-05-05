use super::*;
use std::fs;
use tempfile::TempDir;

/// Build a fake project containing:
///   - `<root>/compile_commands.json`
///   - `<root>/Qt/include/QtCore/QObject` (so `-I<root>/Qt/include` resolves)
///   - `<root>/boost/include/boost/version.hpp`
///   - `<root>/internal_sdk/include/megacorp.h`
fn fixture_project_with_cc_json() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Qt/include/QtCore")).unwrap();
    fs::create_dir_all(root.join("boost/include/boost")).unwrap();
    fs::create_dir_all(root.join("internal_sdk/include")).unwrap();
    fs::write(root.join("Qt/include/QtCore/QObject"), "#include \"qobject.h\"\n").unwrap();
    fs::write(root.join("Qt/include/QtCore/qobject.h"), "class QObject {};\n").unwrap();
    fs::write(root.join("boost/include/boost/version.hpp"), "#define BOOST_VERSION 108300\n").unwrap();
    fs::write(root.join("internal_sdk/include/megacorp.h"), "void mc_init(void);\n").unwrap();
    tmp
}

fn write_cc_json(root: &std::path::Path, body: &str) {
    fs::write(root.join("compile_commands.json"), body).unwrap();
}

#[test]
fn extracts_minus_i_from_command_string() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt_include = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let boost_include = root.join("boost/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{
    "directory": "{0}",
    "file": "{0}/main.cpp",
    "command": "g++ -I{1} -I{2} -DQT_CORE_LIB -O2 -c main.cpp"
}}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt_include,
        boost_include
    );
    write_cc_json(root, &body);

    let roots = discover_from_compile_commands(root);
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| r.root.canonicalize().unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    let qt_canon = root.join("Qt/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let boost_canon = root.join("boost/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    assert!(root_paths.iter().any(|p| p == &qt_canon), "qt include missing; got {root_paths:?}");
    assert!(root_paths.iter().any(|p| p == &boost_canon), "boost include missing; got {root_paths:?}");
}

#[test]
fn extracts_minus_i_from_arguments_array() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let internal = root.join("internal_sdk/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{
    "directory": "{0}",
    "file": "{0}/main.cpp",
    "arguments": ["clang++", "-I{1}", "-isystem", "{0}/Qt/include", "-c", "main.cpp"]
}}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        internal
    );
    write_cc_json(root, &body);

    let roots = discover_from_compile_commands(root);
    let internal_canon = root.join("internal_sdk/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let qt_canon = root.join("Qt/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| r.root.canonicalize().unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(root_paths.iter().any(|p| p == &internal_canon), "internal sdk -I missing");
    assert!(root_paths.iter().any(|p| p == &qt_canon), "qt -isystem missing");
}

#[test]
fn deduplicates_includes_seen_in_multiple_entries() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "g++ -I{1} -c a.cpp" }},
{{ "directory": "{0}", "file": "b.cpp", "command": "g++ -I{1} -c b.cpp" }},
{{ "directory": "{0}", "file": "c.cpp", "command": "g++ -I{1} -c c.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    assert_eq!(roots.len(), 1, "duplicate -I entries must dedupe; got {roots:?}");
}

#[test]
fn skips_nonexistent_paths() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    // -I points at a path that doesn't exist on disk; must not surface.
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "g++ -I{0}/totally-bogus-path -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/")
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    assert!(roots.is_empty(), "non-existent paths must be filtered; got {roots:?}");
}

#[test]
fn relative_include_paths_resolve_against_directory() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "g++ -IQt/include -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/")
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    let qt_canon = root.join("Qt/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| r.root.canonicalize().unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        root_paths.iter().any(|p| p == &qt_canon),
        "relative -IQt/include must resolve against directory; got {root_paths:?}"
    );
}

#[test]
fn locates_compile_commands_in_build_subdir() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("build")).unwrap();
    fs::create_dir_all(root.join("Qt/include/QtCore")).unwrap();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "g++ -I{1} -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt
    );
    fs::write(root.join("build/compile_commands.json"), body).unwrap();
    let roots = discover_from_compile_commands(root);
    assert_eq!(roots.len(), 1, "build/ subdir must be probed; got {roots:?}");
}

#[test]
fn locates_compile_commands_in_cmake_build_subdir() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("cmake-build-debug")).unwrap();
    fs::create_dir_all(root.join("Qt/include/QtCore")).unwrap();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "g++ -I{1} -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt
    );
    fs::write(root.join("cmake-build-debug/compile_commands.json"), body).unwrap();
    let roots = discover_from_compile_commands(root);
    assert_eq!(roots.len(), 1, "cmake-build-* subdir must be probed; got {roots:?}");
}

#[test]
fn no_compile_commands_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let roots = discover_from_compile_commands(tmp.path());
    assert!(roots.is_empty());
}

#[test]
fn malformed_json_returns_empty_without_panic() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("compile_commands.json"), "not valid { json").unwrap();
    let roots = discover_from_compile_commands(tmp.path());
    assert!(roots.is_empty(), "malformed JSON must not panic; got {roots:?}");
}

#[test]
fn project_has_compile_commands_json_detects_root() {
    let tmp = TempDir::new().unwrap();
    assert!(!project_has_compile_commands_json(tmp.path()));
    fs::write(tmp.path().join("compile_commands.json"), "[]").unwrap();
    assert!(project_has_compile_commands_json(tmp.path()));
}

#[test]
fn project_has_compile_commands_json_detects_build_subdir() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("build")).unwrap();
    fs::write(tmp.path().join("build/compile_commands.json"), "[]").unwrap();
    assert!(project_has_compile_commands_json(tmp.path()));
}

#[test]
fn project_has_compile_commands_json_returns_false_when_absent() {
    let tmp = TempDir::new().unwrap();
    assert!(!project_has_compile_commands_json(tmp.path()));
}

#[test]
fn precedence_qt_walker_suppressed_when_compile_commands_present() {
    use crate::ecosystem::{Ecosystem, EcosystemId, QtRuntimeEcosystem};
    use std::collections::HashMap;
    // Point QT_DIR at a real fixture so the walker WOULD return a root
    // if the precedence rule weren't gating it.
    let qt_tmp = TempDir::new().unwrap();
    let qt_include = qt_tmp.path().join("include");
    fs::create_dir_all(qt_include.join("QtCore")).unwrap();
    fs::write(qt_include.join("QtCore/qobject.h"), "class QObject {};\n").unwrap();
    std::env::set_var("BEARWISDOM_QT_DIR", qt_tmp.path());

    let project_tmp = TempDir::new().unwrap();
    fs::write(project_tmp.path().join("compile_commands.json"), "[]").unwrap();

    let manifests: HashMap<EcosystemId, Vec<std::path::PathBuf>> = HashMap::new();
    let active: Vec<EcosystemId> = Vec::new();
    let ctx = LocateContext {
        project_root: project_tmp.path(),
        manifests: &manifests,
        active_ecosystems: &active,
    };
    let roots = <QtRuntimeEcosystem as Ecosystem>::locate_roots(
        &QtRuntimeEcosystem,
        &ctx,
    );
    std::env::remove_var("BEARWISDOM_QT_DIR");
    assert!(
        roots.is_empty(),
        "Qt walker must suppress when compile_commands.json is present; got {roots:?}"
    );
}

#[test]
fn precedence_qt_walker_active_without_compile_commands() {
    use crate::ecosystem::{Ecosystem, EcosystemId, QtRuntimeEcosystem};
    use std::collections::HashMap;
    // Same fixture, but no compile_commands.json — Qt walker SHOULD activate.
    let qt_tmp = TempDir::new().unwrap();
    let qt_include = qt_tmp.path().join("include");
    fs::create_dir_all(qt_include.join("QtCore")).unwrap();
    fs::write(qt_include.join("QtCore/qobject.h"), "class QObject {};\n").unwrap();
    std::env::set_var("BEARWISDOM_QT_DIR", qt_tmp.path());

    let project_tmp = TempDir::new().unwrap();

    let manifests: HashMap<EcosystemId, Vec<std::path::PathBuf>> = HashMap::new();
    let active: Vec<EcosystemId> = Vec::new();
    let ctx = LocateContext {
        project_root: project_tmp.path(),
        manifests: &manifests,
        active_ecosystems: &active,
    };
    let roots = <QtRuntimeEcosystem as Ecosystem>::locate_roots(
        &QtRuntimeEcosystem,
        &ctx,
    );
    std::env::remove_var("BEARWISDOM_QT_DIR");
    assert!(
        !roots.is_empty(),
        "Qt walker must activate when no compile_commands.json — fallback case"
    );
}

#[test]
fn tokenize_command_handles_quoted_paths() {
    // A path with a space in it, double-quoted.
    let argv = tokenize_command(r#"g++ "-IC:/Program Files/Some SDK/include" -c main.cpp"#);
    assert!(
        argv.iter().any(|a| a == "-IC:/Program Files/Some SDK/include"),
        "quoted -I path must come through intact; got {argv:?}"
    );
}
