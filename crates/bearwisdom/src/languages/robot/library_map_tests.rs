use super::library_map::{
    build_robot_library_map, build_robot_resource_basename_map, RobotPythonLibrary,
};
use crate::types::{
    EdgeKind, ExtractedRef, FlowMeta, ParsedFile,
};

fn import_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: Some(target.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}

fn pf(path: &str, refs: Vec<ExtractedRef>) -> ParsedFile {
    let language = if path.ends_with(".py") { "python" } else { "robot" };
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

#[test]
fn direct_library_resolves_to_project_py_file() {
    let parsed = vec![
        pf(
            "atest/resources/TestCheckerLibrary.py",
            Vec::new(),
        ),
        pf(
            "atest/resources/atest_resource.robot",
            vec![import_ref("TestCheckerLibrary")],
        ),
    ];
    let map = build_robot_library_map(&parsed);
    let entry = map
        .get("atest/resources/atest_resource.robot")
        .expect("resource file must have a library entry");
    assert_eq!(entry.len(), 1);
    assert_eq!(
        entry[0],
        RobotPythonLibrary {
            library_name: "TestCheckerLibrary".to_string(),
            py_file_path: "atest/resources/TestCheckerLibrary.py".to_string(),
        }
    );
}

#[test]
fn library_with_explicit_py_extension_resolves_directly() {
    // `Library  KeywordDecorator.py` is a legal Robot import form. The
    // name IS already the file basename and must be matched as-is.
    // Stripping a leading `.`-segment would reduce it to `py` and
    // search for `py.py`, which never exists.
    let parsed = vec![
        pf("atest/testdata/keywords/KeywordDecorator.py", Vec::new()),
        pf(
            "atest/testdata/keywords/foo.robot",
            vec![import_ref("KeywordDecorator.py")],
        ),
    ];
    let map = build_robot_library_map(&parsed);
    let entry = map
        .get("atest/testdata/keywords/foo.robot")
        .expect("must resolve explicit .py library");
    assert!(entry.iter().any(|l|
        l.py_file_path == "atest/testdata/keywords/KeywordDecorator.py"
    ), "got {entry:?}");
}

#[test]
fn dotted_library_name_resolves_via_last_segment() {
    let parsed = vec![
        pf(
            "robot/util/MyLib.py",
            Vec::new(),
        ),
        pf(
            "tests/foo.robot",
            vec![import_ref("robot.util.MyLib")],
        ),
    ];
    let map = build_robot_library_map(&parsed);
    let entry = map.get("tests/foo.robot").expect("must resolve");
    assert_eq!(entry[0].py_file_path, "robot/util/MyLib.py");
}

#[test]
fn unknown_library_is_silently_dropped() {
    let parsed = vec![pf(
        "tests/foo.robot",
        vec![import_ref("SeleniumLibrary")],
    )];
    let map = build_robot_library_map(&parsed);
    // No .py file matches → entry omitted (not an error — external libs
    // are handled by infer_external_namespace).
    assert!(map.get("tests/foo.robot").is_none());
}

#[test]
fn transitive_library_propagates_through_resource_chain() {
    let parsed = vec![
        pf(
            "atest/resources/TestCheckerLibrary.py",
            Vec::new(),
        ),
        pf(
            "atest/resources/atest_resource.robot",
            vec![import_ref("TestCheckerLibrary")],
        ),
        pf(
            "atest/robot/output/foo.robot",
            vec![import_ref("atest_resource.robot")],
        ),
    ];
    let map = build_robot_library_map(&parsed);
    let entry = map
        .get("atest/robot/output/foo.robot")
        .expect("transitive library must propagate");
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].library_name, "TestCheckerLibrary");
    assert_eq!(entry[0].py_file_path, "atest/resources/TestCheckerLibrary.py");
}

#[test]
fn resource_cycle_does_not_loop_forever() {
    // a.robot ⇄ b.robot — pathological but legal.
    let parsed = vec![
        pf(
            "a.robot",
            vec![import_ref("b.robot")],
        ),
        pf(
            "b.robot",
            vec![import_ref("a.robot")],
        ),
    ];
    // No assertion on contents — just must not infinite-loop.
    let _ = build_robot_library_map(&parsed);
}

#[test]
fn relative_resource_path_resolves_to_indexed_basename() {
    // Robot lets you write `Resource    ../runner/cli_resource.robot`
    // — the extractor stores the literal user-written path. The
    // basename matcher must strip the leading `../<dir>/` segments
    // before lookup, otherwise the chain breaks every time a project
    // organises resources by feature folder.
    let parsed = vec![
        pf("atest/robot/cli/runner/cli_resource.robot", Vec::new()),
        pf(
            "atest/robot/cli/rebot/rebot_cli_resource.robot",
            vec![import_ref("../runner/cli_resource.robot")],
        ),
        pf("py/Lib.py", Vec::new()),
        pf(
            "atest/robot/cli/runner/cli_resource.robot",
            // already declared above; deduped by HashMap insertion in
            // the basename builder. Re-declaring here just to attach
            // its own Library import for the transitive walk below.
            vec![import_ref("Lib")],
        ),
    ];
    // Re-build with the intended structure (the duplicate above is a
    // type-system convenience; the second insert wins, carrying the
    // Library import).
    let map = build_robot_library_map(&parsed);
    let entry = map
        .get("atest/robot/cli/rebot/rebot_cli_resource.robot")
        .expect("relative path import must resolve transitively");
    assert!(entry.iter().any(|l| l.py_file_path == "py/Lib.py"),
        "transitive Library through `../runner/...` import; got {entry:?}");
}

#[test]
fn library_in_same_dir_wins_over_distant_match() {
    let parsed = vec![
        pf("vendor/TestCheckerLibrary.py", Vec::new()),
        pf("atest/resources/TestCheckerLibrary.py", Vec::new()),
        pf(
            "atest/resources/atest_resource.robot",
            vec![import_ref("TestCheckerLibrary")],
        ),
    ];
    let map = build_robot_library_map(&parsed);
    let entry = map
        .get("atest/resources/atest_resource.robot")
        .expect("must resolve");
    assert_eq!(entry[0].py_file_path, "atest/resources/TestCheckerLibrary.py",
        "same-dir match must beat distant candidate");
}

#[test]
fn builtin_py_auto_imports_into_every_robot_file() {
    // Project vendors the framework's BuiltIn.py at the canonical path.
    // Every robot file should get an implicit binding even when it
    // declares no Library imports.
    let parsed = vec![
        pf("src/robot/libraries/BuiltIn.py", Vec::new()),
        pf("tests/no_imports.robot", Vec::new()),
        pf(
            "tests/with_resource.robot",
            vec![import_ref("helpers.robot")],
        ),
        pf("tests/helpers.robot", Vec::new()),
    ];
    let map = build_robot_library_map(&parsed);

    let bare = map
        .get("tests/no_imports.robot")
        .expect("BuiltIn must auto-inject even with no explicit imports");
    assert!(
        bare.iter().any(|l|
            l.library_name == "BuiltIn"
                && l.py_file_path == "src/robot/libraries/BuiltIn.py"
        ),
        "expected BuiltIn auto-import; got {bare:?}"
    );

    // The helper resource also gets BuiltIn (for keywords IT defines).
    let helpers = map
        .get("tests/helpers.robot")
        .expect("BuiltIn auto-injects into resource files too");
    assert!(helpers.iter().any(|l| l.library_name == "BuiltIn"));
}

#[test]
fn no_builtin_py_means_no_auto_import() {
    // Application project that uses Robot at runtime but doesn't vendor
    // the framework. There's no BuiltIn.py in the project tree, so the
    // resolver shouldn't pretend there is one.
    let parsed = vec![
        pf("tests/no_imports.robot", Vec::new()),
    ];
    let map = build_robot_library_map(&parsed);
    assert!(
        map.get("tests/no_imports.robot").is_none(),
        "no .py files in project ⇒ no library entry at all"
    );
}

#[test]
fn basename_map_resolves_resource_imports_to_full_paths() {
    let parsed = vec![
        pf("atest/resources/atest_resource.robot", Vec::new()),
        pf("atest/robot/output/foo.robot", Vec::new()),
    ];
    let map = build_robot_resource_basename_map(&parsed);
    assert_eq!(
        map.get("atest_resource.robot")
            .map(|v| v.as_slice()),
        Some(&["atest/resources/atest_resource.robot".to_string()][..]),
    );
    assert_eq!(
        map.get("foo.robot").map(|v| v.as_slice()),
        Some(&["atest/robot/output/foo.robot".to_string()][..]),
    );
}

#[test]
fn basename_map_collects_all_candidates_sorted_lexicographically() {
    // Two project files with the same basename — both candidates kept
    // so the resolver can prefer the importer-dir match. Lex-sorted
    // for determinism (stable across runs).
    let parsed = vec![
        pf("vendor/helpers.robot", Vec::new()),
        pf("atest/resources/helpers.robot", Vec::new()),
    ];
    let map = build_robot_resource_basename_map(&parsed);
    assert_eq!(
        map.get("helpers.robot").map(|v| v.as_slice()),
        Some(&[
            "atest/resources/helpers.robot".to_string(),
            "vendor/helpers.robot".to_string(),
        ][..]),
        "got {map:?}",
    );
}

#[test]
fn pick_resource_prefers_same_directory_when_basenames_collide() {
    use super::library_map::pick_resource_for_importer;
    let candidates = vec![
        "atest/robot/standard_libraries/telnet/telnet_resource.robot".to_string(),
        "atest/testdata/standard_libraries/telnet/telnet_resource.robot".to_string(),
    ];
    let importer = "atest/testdata/standard_libraries/telnet/configuration.robot";
    assert_eq!(
        pick_resource_for_importer(&candidates, importer),
        Some("atest/testdata/standard_libraries/telnet/telnet_resource.robot"),
    );
}

#[test]
fn pick_resource_falls_back_to_first_when_no_same_dir_match() {
    use super::library_map::pick_resource_for_importer;
    let candidates = vec![
        "atest/resources/helpers.robot".to_string(),
        "vendor/helpers.robot".to_string(),
    ];
    let importer = "tests/some_test.robot";
    assert_eq!(
        pick_resource_for_importer(&candidates, importer),
        Some("atest/resources/helpers.robot"),
    );
}

#[test]
fn external_files_are_excluded_from_inputs() {
    // ext: paths represent externals (node_modules, .m2, etc.). The
    // resolver shouldn't treat them as project files.
    let parsed = vec![
        pf("ext:Lib.py", Vec::new()),
        pf("Lib.py", Vec::new()),
        pf("foo.robot", vec![import_ref("Lib")]),
    ];
    let map = build_robot_library_map(&parsed);
    assert_eq!(map.get("foo.robot").unwrap()[0].py_file_path, "Lib.py");
}
