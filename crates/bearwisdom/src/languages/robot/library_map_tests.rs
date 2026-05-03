use super::library_map::{build_robot_library_map, RobotPythonLibrary};
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
