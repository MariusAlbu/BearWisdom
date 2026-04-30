use super::*;

#[test]
fn ecosystem_identity() {
    let e = RobotSeleniumLibraryEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert!(Ecosystem::languages(&e).contains(&"robot"));
}

#[test]
fn uses_demand_driven() {
    assert!(RobotSeleniumLibraryEcosystem.uses_demand_driven_parse());
}

#[test]
fn non_selenium_project_returns_empty_roots() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roots = ExternalSourceLocator::locate_roots(&RobotSeleniumLibraryEcosystem, dir.path());
    assert!(
        roots.is_empty(),
        "locate_roots must return empty when no SeleniumLibrary import is found"
    );
}

#[test]
fn selenium_project_returns_root() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let robot_file = dir.path().join("test.robot");
    std::fs::File::create(&robot_file)
        .unwrap()
        .write_all(b"*** Settings ***\nLibrary    SeleniumLibrary\n\n*** Test Cases ***\nLogin\n    Open Browser    https://example.com    chrome\n")
        .unwrap();
    let roots = ExternalSourceLocator::locate_roots(&RobotSeleniumLibraryEcosystem, dir.path());
    assert_eq!(roots.len(), 1, "locate_roots must return sentinel for SeleniumLibrary projects");
}

#[test]
fn synthesize_contains_core_selenium_keywords() {
    let files = synthesize_library();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    for kw in [
        "Open Browser",
        "Close Browser",
        "Click Element",
        "Input Text",
        "Get Text",
        "Wait Until Element Is Visible",
        "Page Should Contain",
        "Select From List By Value",
        "Execute Javascript",
        "Capture Page Screenshot",
    ] {
        assert!(names.contains(&kw), "SeleniumLibrary keyword `{kw}` must be synthesized");
    }
}

#[test]
fn all_symbols_are_function_kind() {
    let files = synthesize_library();
    for f in &files {
        for sym in &f.symbols {
            assert_eq!(
                sym.kind,
                crate::types::SymbolKind::Function,
                "SeleniumLibrary keyword `{}` should be Function kind",
                sym.name
            );
        }
    }
}

#[test]
fn parallel_vecs_consistent() {
    let files = synthesize_library();
    for f in &files {
        assert_eq!(
            f.symbols.len(),
            f.symbol_origin_languages.len(),
            "symbol_origin_languages must match symbols for {}",
            f.path
        );
        assert_eq!(
            f.symbols.len(),
            f.symbol_from_snippet.len(),
            "symbol_from_snippet must match symbols for {}",
            f.path
        );
    }
}

#[test]
fn each_file_has_exactly_one_symbol() {
    let files = synthesize_library();
    for f in &files {
        assert_eq!(
            f.symbols.len(),
            1,
            "synthetic file {} should have exactly one symbol",
            f.path
        );
    }
}

#[test]
fn parse_metadata_only_returns_library() {
    let e = RobotSeleniumLibraryEcosystem;
    let root = synthetic_dep_root();
    let files = Ecosystem::parse_metadata_only(&e, &root).expect("must return Some");
    assert!(!files.is_empty());
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();
    assert!(names.contains(&"Open Browser"));
    assert!(names.contains(&"Get WebElements"));
}
