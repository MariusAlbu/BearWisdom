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
fn tu_file_set_collects_canonical_tu_paths() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("a.cpp"), "// a\n").unwrap();
    fs::write(src_dir.join("b.cpp"), "// b\n").unwrap();
    fs::write(src_dir.join("c.cpp"), "// c\n").unwrap();
    let body = format!(
        r#"[
{{"directory": "{root_s}", "file": "src/a.cpp", "command": "cc -c src/a.cpp"}},
{{"directory": "{root_s}", "file": "src/b.cpp", "command": "cc -c src/b.cpp"}}
]"#,
        root_s = root.to_string_lossy().replace('\\', "/")
    );
    write_cc_json(root, &body);
    let tus = tu_file_set(root).expect("tu_file_set should return Some when compile_commands.json has entries");
    let canon = |p: &std::path::Path| p.canonicalize().unwrap();
    assert!(tus.contains(&canon(&src_dir.join("a.cpp"))));
    assert!(tus.contains(&canon(&src_dir.join("b.cpp"))));
    assert!(!tus.contains(&canon(&src_dir.join("c.cpp"))),
        "c.cpp is on disk but absent from compile_commands.json — must NOT be in TU set");
}

#[test]
fn tu_file_set_returns_none_when_compile_commands_absent() {
    let tmp = TempDir::new().unwrap();
    assert!(tu_file_set(tmp.path()).is_none());
}

#[test]
fn tu_file_set_returns_none_when_entries_have_no_file_field() {
    // Edge case: compile_commands.json with entries that omit `file`
    // (technically malformed but seen in the wild from custom build
    // wrappers). Filter must not return an empty allowlist that would
    // wipe out every walked source file.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_cc_json(root, r#"[{"directory": ".", "command": "cc -c x.c"}]"#);
    assert!(tu_file_set(root).is_none(),
        "no `file` fields means no TU set — caller treats as no-op");
}

#[test]
fn tu_file_set_handles_absolute_file_paths() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("a.cpp"), "// a\n").unwrap();
    let abs = src_dir.join("a.cpp").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[{{"directory": "/somewhere/else", "file": "{abs}", "command": "cc -c {abs}"}}]"#
    );
    write_cc_json(root, &body);
    let tus = tu_file_set(root).unwrap();
    assert!(tus.contains(&src_dir.join("a.cpp").canonicalize().unwrap()));
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

#[test]
fn tokenize_command_preserves_unquoted_windows_backslashes() {
    // CMake on Windows emits unquoted backslash paths in the `command` field.
    // JSON-decoded, they look like `cl.exe -IF:\Work\Foo`. The tokenizer must
    // pass `\` through literally outside quotes; otherwise every Windows path
    // collapses (`-IF:\Work\Foo` → `-IF:WorkFoo`, silently breaking discovery).
    let argv = tokenize_command("cl.exe -IF:\\Work\\Projects\\Foo -c main.cpp");
    assert!(
        argv.iter().any(|a| a == "-IF:\\Work\\Projects\\Foo"),
        "unquoted Windows path must survive tokenization; got {argv:?}"
    );
}

#[test]
fn tokenize_command_preserves_quoted_windows_backslashes() {
    // Inside double quotes, `\` is only an escape for `\"` and `\\`. Backslash
    // before any other character is a literal — Windows paths survive.
    let argv = tokenize_command(
        "cl.exe \"-IC:\\Program Files\\Qt\\include\" -c main.cpp",
    );
    assert!(
        argv.iter().any(|a| a == "-IC:\\Program Files\\Qt\\include"),
        "quoted Windows path must survive tokenization; got {argv:?}"
    );
}

#[test]
fn tokenize_command_still_escapes_quote_inside_double_quotes() {
    // `\"` inside a double-quoted string is the only POSIX-shell escape that
    // matters for compile_commands. Make sure our narrowed rule still honors it.
    let argv = tokenize_command(r#"g++ "-DGREETING=\"hi\"" -c main.cpp"#);
    assert!(
        argv.iter().any(|a| a == r#"-DGREETING="hi""#),
        "embedded \\\" must still be unescaped inside double quotes; got {argv:?}"
    );
}

#[test]
fn tokenize_command_handles_backslash_quote_outside_double_quotes() {
    // CMake on MSBuild emits defines like:
    //   -DQT_TESTCASE_BUILDDIR=\"F:/path\"
    // (literal backslashes before quotes in the actual command string,
    //  after JSON decoding). The `\"` is the escape — it must NOT toggle
    //  in_double, otherwise every flag after it gets glued into one
    //  mega-token until the matching plain `"`. On a single-line command,
    //  that plain `"` never arrives, so flags like
    //  `-external:IC:\Qt\...\include\QtTest` get silently swallowed.
    //
    // This is the exact shape that broke KeePassXC's QtTest discovery and
    // produced ~10K Qt-test-macro unresolved refs (QCOMPARE/QVERIFY/...).
    let cmd = "cl.exe -DQT_TESTCASE_BUILDDIR=\\\"F:/path\\\" -external:IC:\\Qt\\include\\QtTest -c file.cpp";
    let argv = tokenize_command(cmd);
    assert!(
        argv.iter().any(|a| a == "-external:IC:\\Qt\\include\\QtTest"),
        "QtTest -external:I flag must NOT be eaten by an unterminated double quote opened by the preceding `=\\\"...\\\"` define; got {argv:?}"
    );
    // The define itself comes through as one token with literal embedded
    // quotes, exactly as cl.exe would receive it.
    assert!(
        argv.iter().any(|a| a == "-DQT_TESTCASE_BUILDDIR=\"F:/path\""),
        "define with backslash-escaped quotes should land as one token \
         with literal embedded quotes; got {argv:?}"
    );
}

#[test]
fn extracts_external_i_combined_form() {
    // MSVC's `-external:IC:\foo\include` treats the path as third-party
    // includes. KeePassXC and most CMake-on-MSBuild projects use this form
    // for Qt and vcpkg roots. Without parsing it, every SDK header is
    // invisible.
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let vcpkg = root.join("boost/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "cl.exe -external:I{1} -external:I{2} -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt,
        vcpkg
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    let qt_canon = root.join("Qt/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let vcpkg_canon = root.join("boost/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| r.root.canonicalize().unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(root_paths.iter().any(|p| p == &qt_canon),
        "qt -external:I missing; got {root_paths:?}");
    assert!(root_paths.iter().any(|p| p == &vcpkg_canon),
        "vcpkg -external:I missing; got {root_paths:?}");
}

#[test]
fn extracts_external_i_separated_form() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "arguments": ["cl.exe", "-external:I", "{1}", "-c", "a.cpp"] }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    let qt_canon = root.join("Qt/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| r.root.canonicalize().unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(root_paths.iter().any(|p| p == &qt_canon),
        "qt -external:I (separated) missing; got {root_paths:?}");
}

#[test]
fn extracts_external_i_slash_form() {
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "cl.exe /external:I{1} -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    let qt_canon = root.join("Qt/include").canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| r.root.canonicalize().unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(root_paths.iter().any(|p| p == &qt_canon),
        "qt /external:I missing; got {root_paths:?}");
}

#[test]
fn external_warning_flags_are_not_consumed_as_paths() {
    // `-external:W0`, `-external:env:VAR`, and `-external:templates-` look
    // like the path flag but are not. The extractor must only fire on
    // `-external:I`. If the W0 form were consumed, push_path("0", dir, out)
    // would add a non-existent dir (filtered later by is_dir) — silently
    // wasteful, not visible regression. Test belt-and-braces.
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt = root.join("Qt/include").to_string_lossy().replace('\\', "/");
    let body = format!(
        r#"[
{{ "directory": "{0}", "file": "a.cpp", "command": "cl.exe -external:W0 -external:templates- -external:I{1} -c a.cpp" }}
]"#,
        root.to_string_lossy().replace('\\', "/"),
        qt
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    // Only qt/include should resolve. The W0 / templates- args refer to
    // paths "0" and "-" which don't exist on disk.
    assert_eq!(roots.len(), 1, "non-include -external: flags must not produce roots; got {roots:?}");
}

#[test]
fn extracts_paths_from_realistic_msbuild_command_string() {
    // End-to-end: full MSBuild-shape command string with backslash paths +
    // -external:I + -I + defines + /std flag, exactly like KeePassXC's
    // compile_commands.json. Both the tokenizer and extractor must
    // cooperate.
    let tmp = fixture_project_with_cc_json();
    let root = tmp.path();
    let qt_path = root.join("Qt/include");
    let qt_core_path = root.join("Qt/include/QtCore");
    let internal_path = root.join("internal_sdk/include");
    let qt = qt_path.to_string_lossy().to_string();
    let qt_core = qt_core_path.to_string_lossy().to_string();
    let internal = internal_path.to_string_lossy().to_string();
    let body = format!(
        r#"[
{{
    "directory": "{dir}",
    "file": "{dir}/a.cpp",
    "command": "cl.exe /nologo /TP -DQT_CORE_LIB -DQT_NO_EXCEPTIONS -I{internal} -external:I{qt} -external:I{qt_core} -external:W0 /std:c++20 /c a.cpp"
}}
]"#,
        dir = root.to_string_lossy().replace('\\', "/"),
        internal = internal.replace('\\', "/"),
        qt = qt.replace('\\', "/"),
        qt_core = qt_core.replace('\\', "/")
    );
    write_cc_json(root, &body);
    let roots = discover_from_compile_commands(root);
    let canon = |p: &std::path::Path| p.canonicalize().unwrap().to_string_lossy().replace('\\', "/");
    let qt_canon = canon(&root.join("Qt/include"));
    let qt_core_canon = canon(&root.join("Qt/include/QtCore"));
    let internal_canon = canon(&root.join("internal_sdk/include"));
    let root_paths: Vec<String> = roots
        .iter()
        .map(|r| canon(&r.root))
        .collect();
    assert!(root_paths.iter().any(|p| p == &qt_canon),
        "Qt root from -external:I missing; got {root_paths:?}");
    assert!(root_paths.iter().any(|p| p == &qt_core_canon),
        "Qt/include/QtCore root from -external:I missing; got {root_paths:?}");
    assert!(root_paths.iter().any(|p| p == &internal_canon),
        "Project -I root missing; got {root_paths:?}");
}
